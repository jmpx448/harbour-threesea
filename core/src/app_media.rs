// SPDX-License-Identifier: AGPL-3.0-only
//! Files and media in conversations: receiving file messages, downloading their blobs into the
//! encrypted media store, and removing media together with their messages.
//!
//! A file message's `body` (UI JSON) carries `fileState` / `thumbnailState`:
//! 0 on the server, 1 downloading, 2 on the device, 3 failed; outgoing ones also `uploadState`:
//! 1 uploading, 2 uploaded (the message can be sent), 3 failed. `fileOnDemand`: the user stopped its
//! download or cleaned the file up, so it comes back only when asked for. `uploadRetry`, `fileRetry` and
//! `thumbnailRetry`: how often that transfer failed and when it may try again by itself;
//! `uploadedFile`: when the file part went up, if the rest of the upload didn't.
//!
//! Sending: the file is first stored in the (encrypted) media store, then uploaded from there,
//! so a failed upload can be retried after a restart without the original file.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
};

use libthreema::model::message::{FileMessage, FileRenderingType};
use rusqlite::{OptionalExtension as _, named_params, params};
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
    web::images,
};

pub const STATE_REMOTE: i64 = 0;
pub const STATE_LOADING: i64 = 1;
pub const STATE_LOCAL: i64 = 2;
pub const STATE_FAILED: i64 = 3;

// The indexes of `messages::MIGRATION_LOOKUP_INDEXES` and the queries they serve spell these out.
const _: () = assert!(STATE_REMOTE == 0 && STATE_LOADING == 1);

/// The pauses before a failed transfer tries again by itself, in minutes: as many automatic tries
/// as there are pauses, on top of the first one. They only cost data on a link that keeps breaking
/// off, so a large file doesn't get to do that for long. A tap always tries, and starts them over.
const RETRY_PAUSES_MINUTES: [i64; 5] = [1, 5, 30, 120, 480];
const MINUTE_MS: i64 = 60_000;
/// The longest pause.
const RETRY_PAUSE_MAX_MS: i64 = RETRY_PAUSES_MINUTES[RETRY_PAUSES_MINUTES.len() - 1] * MINUTE_MS;
const UPLOAD_RETRY: &str = "$.uploadRetry";
const UPLOADED_FILE: &str = "$.uploadedFile";

/// How long an uploaded file part is good for a later try of the rest of its upload. How long the
/// server keeps a blob nobody fetched isn't on record, so this stays well on the safe side.
const UPLOADED_FILE_KEPT_MS: i64 = 24 * 60 * MINUTE_MS;

/// What goes in a message's retry field after a failure, the `tries`-th in a row: `at`, the time of
/// the next try by itself, stays empty when there won't be one.
fn retry_after_failure(tries: i64, transient: bool, now: i64) -> Value {
    let pause = usize::try_from(tries - 1)
        .ok()
        .and_then(|index| RETRY_PAUSES_MINUTES.get(index))
        .filter(|_| transient);
    json!({"tries": tries, "at": pause.map(|minutes| now + minutes * MINUTE_MS)})
}

/// Whether a try planned for `at` is due. A time further ahead than any pause comes from a clock
/// that went back since, and nothing should wait for that clock to catch up.
fn retry_due(at: i64, now: i64) -> bool {
    at <= now || at > now + RETRY_PAUSE_MAX_MS
}

/// What the core loop has to do about failed transfers.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TransferRetries {
    /// Outgoing file messages whose upload may try again now.
    pub uploads: Vec<i64>,
    /// Parts whose download may try again now.
    pub downloads: Vec<(i64, MediaPart)>,
    /// When the next one comes due.
    pub next_at: Option<i64>,
}

impl TransferRetries {
    /// `at` for a try: due, or to look at again by then.
    fn sort_in(&mut self, at: i64, now: i64) -> bool {
        let due = retry_due(at, now);
        if !due {
            self.next_at = Some(self.next_at.map_or(at, |next| next.min(at)));
        }
        due
    }
}

/// Where an upload's blobs go at the moment.
#[derive(Clone, Copy)]
struct UploadTarget {
    /// To the device group's mirror.
    mirror: bool,
    scope: media::BlobScope,
    persist: bool,
}

/// May an upload's next try leave out the file part that went up earlier? Not after a long time,
/// and not if the blobs go another way by now: to another server (a device was linked or removed
/// since), or for others to fetch as well (a notes group that gained a member, as what we sent
/// for ourselves may be gone by the time they ask).
fn uploaded_file_good(body: &Value, now: i64, target: UploadTarget) -> bool {
    let uploaded = &body["uploadedFile"];
    uploaded["mirror"].as_bool() == Some(target.mirror)
        && uploaded["scope"].as_str() == Some(target.scope.as_str())
        && uploaded["persist"].as_bool() == Some(target.persist)
        && uploaded["at"]
            .as_i64()
            .is_some_and(|at| at <= now && now - at < UPLOADED_FILE_KEPT_MS)
}

fn state_field(part: MediaPart) -> &'static str {
    match part {
        MediaPart::File => "$.fileState",
        MediaPart::Thumbnail => "$.thumbnailState",
    }
}

fn retry_field(part: MediaPart) -> &'static str {
    match part {
        MediaPart::File => "$.fileRetry",
        MediaPart::Thumbnail => "$.thumbnailRetry",
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
    /// See `sendFile`'s `keepMetadata`.
    pub keep_metadata: bool,
}

/// In a file message's body: the blob's nonce, if it came as a deprecated image (hex).
pub(crate) const LEGACY_NONCE: &str = "legacyNonce";

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
            legacy_nonce: None,
        },
    })
}

/// The UI JSON of a received file message (blob keys go to the `files` table, not here).
pub fn file_body(file: &FileMessage) -> Value {
    let mut body = json!({
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
    });
    if let Some(nonce) = file.metadata.legacy_nonce {
        body[LEGACY_NONCE] = data_encoding::HEXLOWER.encode(&nonce).into();
    }
    body
}

/// The blob keys of a received file message.
pub fn file_keys(file: &FileMessage) -> FileKeys {
    FileKeys {
        key: Zeroizing::new(file.key),
        blob_id: Some(file.blob_id),
        thumbnail_blob_id: file.thumbnail_blob_id,
    }
}

/// A message's age for retention (SQL; `:now` the current time): when it arrived. Except that a
/// `created_at` more than a day after its arrival, and no longer in the future, wins: this phone's
/// clock was far behind when it came, and once corrected the message would look old and could go
/// before anyone read it. A sender's clock ahead can only make its message stay longer this way.
const AGE_SQL: &str = "COALESCE(CASE WHEN created_at > received_at + 86400000
                                      AND created_at <= :now
                                THEN created_at ELSE received_at END, created_at)";

impl Store {
    /// File messages that reached this device before `before` (ms) whose file is on the device
    /// and may still be on a server: not starred, not still on their way out (their upload reads
    /// the file), and with a blob. A list's own copy or an unsent imported file has none, so its
    /// bytes here are the only ones. Age by arrival: an incoming `created_at` is the sender's clock.
    pub fn files_before(
        &self,
        before: i64,
        spare_chats: &[String],
    ) -> Result<Vec<i64>, StoreError> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT id FROM messages WHERE kind = 'file' AND starred = 0
               AND {AGE_SQL} < :before
               AND json_extract(body, '$.fileState') = :local
               AND NOT (outgoing = 1 AND state = :pending)
               AND conversation NOT IN (SELECT value FROM json_each(:spare_chats))
               AND EXISTS (SELECT 1 FROM files
                           WHERE files.message = messages.id AND files.blob_id IS NOT NULL)"
        ))?;
        let rows = statement.query_map(
            named_params! {
                ":now": crate::messages::now_ms(),
                ":before": before,
                ":local": STATE_LOCAL,
                ":pending": crate::messages::OutgoingState::Pending as i64,
                ":spare_chats": json!(spare_chats).to_string(),
            },
            |row| row.get(0),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The user removed a message's file to free space: a fetch, on request only, may bring it back.
    pub fn set_file_cleaned(&self, message: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE messages
             SET body = json_set(body, '$.fileState', ?2, '$.fileOnDemand', json('true')),
                 updated_at = ?3
             WHERE id = ?1 AND kind = 'file'",
            params![message, STATE_REMOTE, now_ms()],
        )?;
        Ok(())
    }

    /// Delete the messages that reached this device before `before` (ms), except starred ones,
    /// messages still on their way out and polls that are still open (Android's auto-delete keeps
    /// those two as well: a message nobody has seen and a vote still running are not old news).
    /// Status lines go by the same rule, as on Android: who called when is what someone setting
    /// a period for privacy wants gone. Returns how many.
    ///
    /// Age by arrival, as on Android: an incoming `created_at` is the sender's clock, which could
    /// make a late or backdated message vanish before anyone saw it. Rows with no arrival time
    /// (ours, imported ones, status lines) have only the one clock.
    ///
    /// `spare_notes`: leave notes groups alone (a group whose only member is us, the same test as
    /// `upload_target`). Keep messages does, like Android's auto-delete: nobody else has a copy
    /// of a notebook, and a period chosen for chats shouldn't eat it. Clean up doesn't, again like
    /// Android: there the user asks for this, now, in every chat.
    ///
    /// `spare_chats`: chats left out altogether (Clean up: private chats while they are hidden).
    pub fn delete_messages_before(
        &self,
        before: i64,
        spare_notes: bool,
        spare_chats: &[String],
    ) -> Result<usize, StoreError> {
        let spare_chats = json!(spare_chats).to_string();
        let old = format!(
            "{AGE_SQL} < :before AND starred = 0
             AND NOT (outgoing = 1 AND state = :pending)
             AND NOT (kind = 'poll' AND json_extract(body, '$.state') = 0)
             AND NOT (:spare_notes AND conversation IN
                      (SELECT key FROM groups WHERE left_group = 0 AND members = '[]'))
             AND conversation NOT IN (SELECT value FROM json_each(:spare_chats))"
        );
        let condition = named_params! {
            ":now": crate::messages::now_ms(),
            ":before": before,
            ":pending": crate::messages::OutgoingState::Pending as i64,
            ":spare_notes": spare_notes,
            ":spare_chats": spare_chats,
        };
        let transaction = self.conn.unchecked_transaction()?;
        // A pinned chat or one with a draft stays in the list when its last row goes: otherwise
        // the pin and the draft would be out of sight. It stays where its newest message put it.
        // Other 1:1 chats leave the list, unlike on Android, which keeps them all empty: a period
        // chosen for privacy shouldn't leave a list of who wrote.
        let kept: Vec<(String, i64)> = transaction
            .prepare(&format!(
                "SELECT conversation, MAX(COALESCE(received_at, created_at)) FROM messages
                 WHERE {old} AND conversation IN
                       (SELECT conversation FROM pinned_chats
                        UNION SELECT substr(key, 7) FROM settings WHERE key LIKE 'draft:%')
                 GROUP BY conversation"
            ))?
            .query_map(condition, |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let removed =
            transaction.execute(&format!("DELETE FROM messages WHERE {old}"), condition)?;
        for (conversation, at) in kept {
            transaction.execute(
                "INSERT OR REPLACE INTO emptied_chats (conversation, at)
                 SELECT ?1, ?2 WHERE NOT EXISTS (SELECT 1 FROM messages WHERE conversation = ?1)",
                params![conversation, at],
            )?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    /// Set `fileState`/`thumbnailState` of a file message; returns the changed message. A download
    /// under way keeps count of its tries; anything else ends them.
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
        if state != STATE_LOADING {
            self.forget_transfer_tries(message, retry_field(part))?;
        }
        self.message(message)
    }

    /// The user stopped a file's download: it waits for a tap again, and doesn't come by itself.
    pub fn set_download_cancelled(&self, message: i64) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, '$.fileOnDemand', json('true'))
             WHERE id = ?1 AND kind = 'file'",
            [message],
        )?;
        self.set_media_state(message, MediaPart::File, STATE_REMOTE)
    }

    /// A part's download failed. `transient`: it may try again by itself after a pause, as long as
    /// it has tries left.
    pub fn set_media_failed(
        &self,
        message: i64,
        part: MediaPart,
        transient: bool,
        now: i64,
    ) -> Result<Option<Message>, StoreError> {
        let tries = self.transfer_tries(message, retry_field(part))? + 1;
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, ?2, ?3, ?4, json(?5)), updated_at = ?6
             WHERE id = ?1 AND kind = 'file'",
            params![
                message,
                state_field(part),
                STATE_FAILED,
                retry_field(part),
                retry_after_failure(tries, transient, now).to_string(),
                now
            ],
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

    /// Set `uploadState` of an outgoing file message. Once it is up, what was noted on the way
    /// there is over.
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
        if state == STATE_LOCAL {
            self.conn.execute(
                "UPDATE messages SET body = json_remove(body, ?2, ?3) WHERE id = ?1 AND kind = 'file'",
                params![message, UPLOAD_RETRY, UPLOADED_FILE],
            )?;
        }
        self.message(message)
    }

    /// The upload failed. `transient`: it may try again by itself after a pause, as long as it
    /// has tries left.
    pub fn set_upload_failed(
        &self,
        message: i64,
        transient: bool,
        now: i64,
    ) -> Result<Option<Message>, StoreError> {
        let tries = self.transfer_tries(message, UPLOAD_RETRY)? + 1;
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, '$.uploadState', ?2, ?3, json(?4)),
                                 updated_at = ?5
             WHERE id = ?1 AND kind = 'file' AND outgoing = 1",
            params![
                message,
                STATE_FAILED,
                UPLOAD_RETRY,
                retry_after_failure(tries, transient, now).to_string(),
                now
            ],
        )?;
        self.message(message)
    }

    fn transfer_tries(&self, message: i64, retry_field: &str) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT json_extract(body, ?2 || '.tries') FROM messages WHERE id = ?1",
                params![message, retry_field],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .unwrap_or(0))
    }

    /// The user asked for this transfer: its tries start over.
    fn forget_transfer_tries(&self, message: i64, retry_field: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE messages SET body = json_remove(body, ?2)
             WHERE id = ?1 AND kind = 'file' AND json_type(body, ?2) IS NOT NULL",
            params![message, retry_field],
        )?;
        Ok(())
    }

    /// A file part is up, whatever becomes of the rest of the upload: a later try needn't send
    /// it again.
    fn keep_uploaded_file(
        &self,
        message: i64,
        keys: &FileKeys,
        now: i64,
        target: UploadTarget,
    ) -> Result<(), StoreError> {
        self.set_file_keys(message, keys)?;
        self.conn.execute(
            "UPDATE messages SET body = json_set(body, ?2, json(?3)) WHERE id = ?1",
            params![
                message,
                UPLOADED_FILE,
                json!({
                    "at": now,
                    "mirror": target.mirror,
                    "scope": target.scope.as_str(),
                    "persist": target.persist,
                })
                .to_string()
            ],
        )?;
        Ok(())
    }

    /// Outgoing file messages whose upload didn't finish (interrupted or failed earlier).
    pub fn unfinished_uploads(&self) -> Result<Vec<i64>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id FROM messages INDEXED BY messages_uploading
             WHERE kind = 'file' AND outgoing = 1 AND json_valid(body)
                 AND json_extract(body, '$.uploadState') = 1",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What may try again by itself: an upload that failed for a message that never went out, if
    /// it hasn't failed for good, and whose pause is over. One that failed before tries were
    /// counted has no note on it and counts as due.
    ///
    /// Downloads likewise, but only with a note: what failed earlier, and what a restored backup
    /// has no blob left for, waits for a tap as ever. A file part goes by what comes by itself on
    /// this network (`auto`, `limit`: see [`Self::media_to_download`]), a thumbnail always does.
    pub fn transfers_to_retry(
        &self,
        now: i64,
        auto: &[String],
        limit: u64,
    ) -> Result<TransferRetries, StoreError> {
        let mut retries = TransferRetries::default();
        let mut statement = self.conn.prepare(
            "SELECT id, json_extract(body, '$.uploadRetry.tries'),
                    json_extract(body, '$.uploadRetry.at')
             FROM messages WHERE kind = 'file' AND outgoing = 1 AND state = 0
                 AND json_extract(body, '$.uploadState') = ?1",
        )?;
        let rows = statement.query_map([STATE_FAILED], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, Option<i64>>(2)?,
            ))
        })?;
        for row in rows {
            let (id, tries, at) = row?;
            let due = match (tries, at) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some(_), Some(at)) => retries.sort_in(at, now),
            };
            if due {
                retries.uploads.push(id);
            }
        }
        let mut statement = self.conn.prepare(
            "SELECT id, body FROM messages WHERE kind = 'file'
                 AND ((json_extract(body, '$.thumbnailState') = ?1
                       AND json_extract(body, '$.thumbnailRetry.at') IS NOT NULL)
                      OR (json_extract(body, '$.fileState') = ?1
                          AND json_extract(body, '$.fileRetry.at') IS NOT NULL))",
        )?;
        let rows = statement.query_map([STATE_FAILED], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, text) = row?;
            let body: Value = serde_json::from_str(&text).unwrap_or_default();
            for (part, state, retry) in [
                (MediaPart::Thumbnail, "thumbnailState", "thumbnailRetry"),
                (MediaPart::File, "fileState", "fileRetry"),
            ] {
                let Some(at) = body[retry]["at"].as_i64() else {
                    continue;
                };
                if body[state].as_i64() == Some(STATE_FAILED)
                    && (part == MediaPart::Thumbnail
                        || file_comes_by_itself(&body, &text, auto, limit))
                    && retries.sort_in(at, now)
                {
                    retries.downloads.push((id, part));
                }
            }
        }
        Ok(retries)
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
    /// has a fixed 5 MB). Thumbnails are never held back by it, and a file cleaned up by the user
    /// (`fileOnDemand`) comes only when asked for.
    pub fn media_to_download(
        &self,
        auto: &[String],
        limit: u64,
    ) -> Result<Vec<(i64, MediaPart)>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, json_extract(body, '$.thumbnailState'), json_extract(body, '$.fileState'),
                    body
             FROM messages INDEXED BY messages_media_to_come
             WHERE kind = 'file' AND json_valid(body)
                 AND (json_extract(body, '$.thumbnailState') = 0
                      OR json_extract(body, '$.fileState') = 0)
             ORDER BY id",
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
            if file == Some(STATE_REMOTE) {
                let parsed = serde_json::from_str::<Value>(&body).unwrap_or_default();
                if file_comes_by_itself(&parsed, &body, auto, limit) {
                    parts.push((id, MediaPart::File));
                }
            }
        }
        Ok(parts)
    }
}

/// Does this message's file part come without being asked for, on the current network?
fn file_comes_by_itself(parsed: &Value, body: &str, auto: &[String], limit: u64) -> bool {
    let size = parsed["size"].as_u64().unwrap_or(0);
    parsed["fileOnDemand"] != true
        && auto.iter().any(|k| k == download_kind(body))
        && (limit == 0 || size <= limit)
}

/// Is a part of this file message still on the blob server (or the mirror), and to come by itself?
fn awaits_download(body: &str) -> bool {
    let body: Value = serde_json::from_str(body).unwrap_or_default();
    body["thumbnailState"].as_i64() == Some(STATE_REMOTE)
        || (body["fileState"].as_i64() == Some(STATE_REMOTE) && body["fileOnDemand"] != true)
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

/// Media downloads running at once (each holds its blob, then the decrypted data, in memory; a large
/// one takes its share of the transfer memory as well).
static DOWNLOAD_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

/// Whether `message` is still the outgoing file with this key, which is random per file. See
/// [`Download::wanted`].
fn same_file(store: &Store, message: i64, key: &[u8; 32]) -> bool {
    matches!(store.file_keys(message), Ok(Some(keys)) if *keys.key == *key)
}

/// `done` on the blob server says nobody needs this blob any more, so it may be dropped. Only an
/// incoming 1:1 message's blob is done for good. Without multi-device nothing else gets one: a
/// group's blob is still needed by the other members (Android's `shouldMarkAsDone`), and neither
/// is our own 1:1 message's ours to drop when a cleaned-up file is fetched again, since its
/// recipient may not have it yet (there Android does send one). With multi-device the mirror keeps
/// the blob for the other devices (`Local`).
fn done_scope(outgoing: bool, group: bool, multi_device: bool) -> Option<media::BlobScope> {
    if !multi_device && (group || outgoing) {
        return None;
    }
    Some(if !outgoing && !group {
        media::BlobScope::Public
    } else {
        media::BlobScope::Local
    })
}

/// How a deprecated image's blob is encrypted: between sender and receiver, with its own nonce.
#[derive(Clone)]
struct LegacySeal {
    key: Zeroizing<[u8; 32]>,
    nonce: [u8; 24],
}

impl LegacySeal {
    /// The seal of `message`'s file: `Ok(None)` if it has the usual file key, `Err` if it is sealed
    /// and we can't open it (no contact left to derive the key with).
    fn of(store: &Store, message: &Message) -> Result<Option<Self>, ()> {
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        let Some(nonce) = body[LEGACY_NONCE].as_str() else {
            return Ok(None);
        };
        let nonce = data_encoding::HEXLOWER
            .decode(nonce.as_bytes())
            .ok()
            .and_then(|nonce| <[u8; 24]>::try_from(nonce).ok())
            .ok_or(())?;
        let identity = store.identity().ok().flatten().ok_or(())?;
        let contact = store
            .contact(&message.conversation)
            .ok()
            .flatten()
            .ok_or(())?;
        let key = libthreema::common::keys::ClientKey::from(*identity.client_key)
            .derive_shared_secret(&contact.public_key)
            .ok_or(())?;
        Ok(Some(Self { key, nonce }))
    }
}

/// One part to fetch: what the task needs, read before it starts.
struct Download {
    message: i64,
    part: MediaPart,
    key: Zeroizing<[u8; 32]>,
    /// Instead of `key`, for a deprecated image's file.
    seal: Option<LegacySeal>,
    blob_id: libthreema::common::BlobId,
    scope: media::BlobScope,
    done_scope: Option<media::BlobScope>,
    save_to: Option<std::path::PathBuf>,
    /// The file's size as its sender tells it (0 if it doesn't).
    announced_size: usize,
}

impl Download {
    /// The most this part may be, whatever its sender says.
    fn largest(&self) -> usize {
        match self.part {
            MediaPart::File => media::MAX_FILE_DOWNLOAD,
            MediaPart::Thumbnail => media::MAX_PICTURE_DOWNLOAD,
        }
    }

    /// The bytes to set memory aside for and fetch at most, at first. What a sender says of its
    /// file isn't to be trusted, so a blob may never take more than was set aside. Only the small
    /// ones get by on a fixed amount.
    fn first_limit(&self) -> usize {
        self.announced_size
            .saturating_add(16)
            .clamp(media::MAX_PICTURE_DOWNLOAD, self.largest())
    }

    /// Whether the message is still this file. Row ids come back: once the newest message is
    /// deleted, the next one gets its id, and a transfer that outlives its message may touch
    /// nothing then. A deleted file has no keys any more, and another file has others; the sender
    /// picks the key of a file message, so the blob counts too.
    fn wanted(&self, store: &Store) -> bool {
        store.file_keys(self.message).is_ok_and(|keys| {
            keys.is_some_and(|keys| {
                let blob_id = match self.part {
                    MediaPart::File => keys.blob_id,
                    MediaPart::Thumbnail => keys.thumbnail_blob_id,
                };
                *keys.key == *self.key && blob_id.is_some_and(|id| id.0 == self.blob_id.0)
            })
        })
    }

    /// Decrypt what was fetched and put it on the disk, off the core thread. The blob's memory
    /// becomes the file's: at no point is it there twice. A file to go to the Gallery as well goes
    /// there before it is encrypted again.
    async fn seal(
        &self,
        store: &Store,
        media: &Arc<MediaStore>,
        blob: Vec<u8>,
    ) -> Result<media::SealedFile, media::MediaError> {
        let (key, nonce) = match &self.seal {
            Some(seal) => (seal.key.clone(), seal.nonce),
            None => (self.key.clone(), *self.part.nonce()),
        };
        let data =
            media::off_thread(move || media::decrypt_blob_in_place(&key, &nonce, blob)).await??;
        // Now, not any earlier: a chat marked private during the download counts too.
        let gallery = self
            .save_to
            .as_deref()
            .filter(|_| self.wanted(store))
            .and_then(|pictures| GallerySave::of(store, self.message, pictures));
        let media = Arc::clone(media);
        media::off_thread(move || {
            if let Some(gallery) = gallery {
                gallery.write(&data);
            }
            media.seal(data)
        })
        .await?
    }

    /// Whatever came of fetching this part: it ends up on the device or as failed.
    async fn keep(
        &self,
        store: &Store,
        media: &Arc<MediaStore>,
        events: &mpsc::UnboundedSender<CoreEvent>,
        fetched: Result<Vec<u8>, media::MediaError>,
    ) -> bool {
        let sealed = match fetched {
            // Deleted meanwhile: no use in the work, and finish() has nothing to do.
            Ok(blob) if self.wanted(store) => self.seal(store, media, blob).await,
            Ok(_) => Err(media::MediaError::Decrypt),
            Err(error) => Err(error),
        };
        self.finish(store, media, events, sealed)
    }

    /// Keep what was fetched, and say so in the message's state; `true` if it is on the device now.
    /// No await in here: nothing can come between the check and the writes.
    fn finish(
        &self,
        store: &Store,
        media: &MediaStore,
        events: &mpsc::UnboundedSender<CoreEvent>,
        sealed: Result<media::SealedFile, media::MediaError>,
    ) -> bool {
        // Deleted meanwhile (for everyone as well): nothing may stay behind, and the id may be
        // another message's by now.
        if !self.wanted(store) {
            tracing::info!(part = ?self.part, "media download dropped, its message is gone");
            return false;
        }
        let stored = sealed.and_then(|sealed| sealed.keep(media, self.message, self.part));
        let changed = match &stored {
            Ok(()) => store.set_media_state(self.message, self.part, STATE_LOCAL),
            Err(error) => {
                tracing::warn!(%error, part = ?self.part, "media download failed");
                // What is here already stays good, whatever became of the blob.
                if media.has(self.message, self.part) {
                    store.set_media_state(self.message, self.part, STATE_LOCAL)
                } else {
                    store.set_media_failed(self.message, self.part, error.is_transient(), now_ms())
                }
            }
        };
        if let Ok(Some(message)) = changed {
            let _ = events.send(CoreEvent::MessagesChanged(vec![message]));
        }
        stored.is_ok()
    }
}

/// Download one part into the media store, updating its state (events go to the UI).
async fn download(
    store: Rc<Store>,
    https: HttpsClient,
    media: Arc<MediaStore>,
    events: mpsc::UnboundedSender<CoreEvent>,
    job: Download,
) {
    let mut limit = job.first_limit();
    let stored = loop {
        // A few at a time, and a large one as the memory allows: it holds a blob of up to 100 MiB.
        let _memory = media::reserve_transfer_memory(limit).await;
        let _slot = DOWNLOAD_SLOTS.acquire().await;
        if !job.wanted(&store) {
            return;
        }
        let fetched = media::download(&https, &job.blob_id, job.scope, limit).await;
        // Larger than its sender said: it may be, but it waits for that much memory first.
        if matches!(fetched, Err(media::MediaError::TooLarge)) && limit < job.largest() {
            limit = job.largest();
            continue;
        }
        break job.keep(&store, &media, &events, fetched).await;
    };
    if stored && let Some(scope) = job.done_scope {
        media::mark_done(&https, &job.blob_id, scope).await;
    }
}

/// A received photo or video on its way to `<Pictures>/Threesea` (Android's "Auto-save to gallery"),
/// unencrypted, as the user asked. Voice messages and other files stay in the app, and so does
/// everything of a private chat (ADR 0009): [`Self::of`] checks that on the core thread, after the
/// download, and if the mark can't be read, nothing is written. [`Self::write`] is for a blocking
/// thread.
struct GallerySave {
    name: String,
    directory: std::path::PathBuf,
}

impl GallerySave {
    fn of(store: &Store, message: i64, pictures: &std::path::Path) -> Option<Self> {
        let message = store.message(message).ok()??;
        if !matches!(store.is_private(&message.conversation), Ok(false))
            || message.outgoing
            || !matches!(
                crate::messages::media_kind(&message.body),
                "image" | "gif" | "video"
            )
        {
            return None;
        }
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        let name = crate::file_types::gallery_file_name(
            body["name"].as_str(),
            body["mediaType"].as_str().unwrap_or(""),
        )?;
        Some(Self {
            name,
            directory: pictures.join("Threesea"),
        })
    }

    fn write(&self, data: &[u8]) {
        if !crate::file_types::looks_like_gallery_media(data)
            || std::fs::create_dir_all(&self.directory).is_err()
        {
            return;
        }
        let target = save_target(&self.directory, &self.name, data);
        if !target.exists()
            && let Err(error) = std::fs::write(&target, data)
        {
            tracing::warn!(%error, "auto-saving media failed");
        }
    }
}

/// Where a message's blob goes, and whether it stays after a download (`persist`), by Android's
/// `shouldPersistUploadForMessage`. A group's stays, since every member fetches it. A notes group
/// (a group whose only member is us) is read by our own devices alone, so its blob stays on the
/// device group's mirror (`isNotesGroup ? BlobScope.Local : BlobScope.Public`); without
/// multi-device there is no mirror and it goes to the usual blob server. A list's copy isn't
/// uploaded: each member gets a 1:1 message with a blob of its own.
fn upload_target(store: &Store, message: i64) -> (media::BlobScope, bool) {
    let group = store
        .message(message)
        .ok()
        .flatten()
        .and_then(|message| store.group(&message.conversation).ok().flatten());
    match group {
        Some(group) if group.is_notes() => (media::BlobScope::Local, false),
        Some(_) => (media::BlobScope::Public, true),
        None => (media::BlobScope::Public, false),
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
    let Ok(Some(mut keys)) = store.file_keys(message) else {
        return;
    };
    let (scope, persist) = upload_target(&store, message);
    let target = UploadTarget {
        mirror: media::mirror_active(),
        scope,
        persist,
    };
    let result = upload_parts(&store, &media, message, &mut keys, target, async |blob| {
        media::upload_scoped(&https, blob, persist, scope).await
    })
    .await;
    finish_upload(&store, &events, message, &keys, result);
}

/// Send what is still to go up of a stored outgoing file. `keys` end up with the blob ids that came
/// back.
async fn upload_parts(
    store: &Store,
    media: &Arc<MediaStore>,
    message: i64,
    keys: &mut FileKeys,
    target: UploadTarget,
    send: impl AsyncFn(Vec<u8>) -> Result<libthreema::common::BlobId, media::MediaError>,
) -> Result<(), media::MediaError> {
    let body = store
        .message(message)
        .ok()
        .flatten()
        .map_or(Value::Null, |message| {
            serde_json::from_str(&message.body).unwrap_or_default()
        });
    if !uploaded_file_good(&body, now_ms(), target) {
        keys.blob_id = None;
    }
    keys.thumbnail_blob_id = None;
    if keys.blob_id.is_none() {
        // Until it is up, the blob is in memory.
        let size = usize::try_from(body["size"].as_u64().unwrap_or(0)).unwrap_or(usize::MAX);
        let _memory = media::reserve_transfer_memory(size.saturating_add(16)).await;
        // Deleted while it waited: this id may be another file's by now.
        if !same_file(store, message, &keys.key) {
            return Ok(());
        }
        let blob = upload_blob(media, message, MediaPart::File, &keys.key)
            .await?
            .ok_or(media::MediaError::FileGone)?;
        keys.blob_id = Some(send(blob).await?);
        // Deleted meanwhile: this id may be another file's by now.
        if same_file(store, message, &keys.key)
            && let Err(error) = store.keep_uploaded_file(message, keys, now_ms(), target)
        {
            tracing::warn!(%error, "keeping an uploaded file's blob id failed");
        }
    }
    // Deleted meanwhile: the thumbnail under this id may be another file's by now.
    if same_file(store, message, &keys.key)
        && let Some(blob) = upload_blob(media, message, MediaPart::Thumbnail, &keys.key).await?
    {
        keys.thumbnail_blob_id = Some(send(blob).await?);
    }
    Ok(())
}

/// A stored part encrypted for the blob server, on a blocking thread and on the one buffer it was
/// read into; `None` if it isn't on the device.
async fn upload_blob(
    media: &Arc<MediaStore>,
    message: i64,
    part: MediaPart,
    key: &Zeroizing<[u8; 32]>,
) -> Result<Option<Vec<u8>>, media::MediaError> {
    let (media, key) = (Arc::clone(media), key.clone());
    media::off_thread(move || {
        Ok(media
            .read(message, part)?
            .map(|data| media::encrypt_blob_in_place(&key, part, data)))
    })
    .await?
}

/// Record how an upload went. No await in here: nothing can come between the check and the writes.
/// A file deleted during its upload leaves no trace: its blob ids on the next file sent, which got
/// its row id, would send the deleted file in that one's place, and a failure would hold that one
/// back.
fn finish_upload(
    store: &Store,
    events: &mpsc::UnboundedSender<CoreEvent>,
    message: i64,
    keys: &FileKeys,
    result: Result<(), media::MediaError>,
) {
    if !same_file(store, message, &keys.key) {
        tracing::info!("upload dropped, its message is gone");
        return;
    }
    let changed = match result.map(|()| store.set_file_keys(message, keys)) {
        Ok(Ok(())) => store.set_upload_state(message, STATE_LOCAL),
        Ok(Err(error)) => {
            tracing::warn!(%error, "storing blob ids failed");
            store.set_upload_failed(message, false, now_ms())
        }
        // There is nothing left to send: the message fails for good, or its clock would never end.
        Err(media::MediaError::FileGone) => {
            tracing::warn!("upload failed for good, the file is gone");
            if let Err(error) = store.set_outgoing_failed(message) {
                tracing::warn!(%error, "storing the failed state failed");
            }
            store.set_upload_failed(message, false, now_ms())
        }
        Err(error) => {
            tracing::warn!(%error, "upload failed");
            store.set_upload_failed(message, error.is_transient(), now_ms())
        }
    };
    if let Ok(Some(message)) = changed {
        let _ = events.send(CoreEvent::MessagesChanged(vec![message]));
    }
}

/// `days` in milliseconds.
fn days_ms(days: u32) -> i64 {
    i64::from(days) * 86_400_000
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

/// A file to send, ready but for its message: encrypted and on the disk, as what it will be sent as.
pub(crate) struct PreparedFile {
    /// The message's body, without its `uploadState`.
    body: Value,
    caption: Option<String>,
    file: media::SealedFile,
    thumbnail: Option<media::SealedFile>,
}

/// Everything about a file to send that takes long, and needs no store: for a blocking thread.
/// Reads it if it didn't come along, makes a voice recording Opus, takes a video's or a GIF's
/// metadata off, and encrypts it. Nothing is left behind if this or the rest fails.
fn prepare_file(
    media: &MediaStore,
    mut file: OutgoingFile,
    attachments: Vec<Zeroizing<Vec<u8>>>,
) -> Result<PreparedFile, ApiError> {
    let mut attachments = attachments.into_iter();
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
        data = Zeroizing::new(std::fs::read(path).map_err(|e| ApiError::Internal(e.to_string()))?);
    }
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
    // Sent as media it is the picture that counts, not what the file says about itself, and
    // the recipient gets told the real size. What can't be vouched for leaves only if the user
    // said so, and then as what it is: a plain file, as it was picked.
    if file.rendering == "media" && file.media_type.starts_with("video/") {
        match crate::mp4::clean(&mut data) {
            Ok(size) => {
                if let Some((width, height)) = size {
                    file.width = Some(width);
                    file.height = Some(height);
                }
            }
            Err(crate::mp4::NotCleanable) if file.keep_metadata => {
                file.rendering = "file".to_owned();
                file.width = None;
                file.height = None;
            }
            Err(crate::mp4::NotCleanable) => return Err(ApiError::VideoKeepsMetadata),
        }
    }
    // A GIF shown as a picture goes without what its file says about itself too, by its bytes,
    // whichever way it came: from this phone, a favourite, a forward or the browser.
    if file.rendering != "file" && crate::gif::is_gif(&data) {
        let (width, height) =
            crate::gif::clean(&mut data).map_err(|_| ApiError::PictureKeepsMetadata)?;
        file.width = Some(width);
        file.height = Some(height);
    }
    if data.len() > media::MAX_BLOB_SIZE {
        return Err(ApiError::FileTooLarge);
    }
    let name = generic_media_name(&file.rendering, &file.media_type)
        .or(file.name)
        .filter(|name| !name.is_empty());
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
        // Like Android: GIFs and animated WebPs shown as media are marked animated (they play in
        // the chat).
        "animated": (file.rendering == "media"
            && (file.media_type == "image/gif"
                || (file.media_type == "image/webp" && crate::webp::is_animated(&data))))
            .then_some(true),
        "thumbnail": thumbnail.is_some(),
        "thumbnailMediaType": "image/jpeg",
        "fileState": STATE_LOCAL,
        "thumbnailState": if thumbnail.is_some() { STATE_LOCAL } else { STATE_FAILED },
    });
    let seal = |data| {
        media
            .seal(data)
            .map_err(|error| ApiError::Internal(error.to_string()))
    };
    Ok(PreparedFile {
        body,
        caption,
        file: seal(data)?,
        thumbnail: thumbnail.map(seal).transpose()?,
    })
}

/// What a command's work on a blocking thread brought back.
pub(crate) enum FileWork {
    /// It wasn't started: what it was for is gone or hidden by the time its turn came.
    Refused(ApiError),
    Send {
        identity: String,
        prepared: Result<PreparedFile, ApiError>,
    },
    Save {
        conversation: String,
        saved: Result<Value, ApiError>,
    },
}

/// That work, for the core loop to finish on its thread. No `work`: the blocking thread failed.
pub(crate) struct FinishedFileWork {
    request: u64,
    work: Option<FileWork>,
}

/// A file on its way out of the store (see [`App::save_file`]).
struct FileToSave {
    /// Which message it is. A row id can come back, as another message.
    conversation: String,
    message_id: u64,
    name: Option<String>,
    media_type: String,
}

impl FileToSave {
    /// Whether the message is still this one, and the UI may still have its file. For the core
    /// thread, right before the file is read: its plain text is what leaves here, and neither the
    /// lock nor a deletion that came during the wait may be left for the answer to find out.
    fn may_go(&self, store: &Store, media: &MediaStore, id: i64) -> Result<(), ApiError> {
        if !media.shown(id) {
            return Err(ApiError::Locked);
        }
        match store.message(id)? {
            Some(message)
                if message.kind == "file"
                    && message.conversation == self.conversation
                    && message.message_id == self.message_id =>
            {
                Ok(())
            }
            _ => Err(ApiError::Internal("no such file".into())),
        }
    }
}

/// How much memory working on a file takes: it came along, or it is read from `path`.
fn file_work_size(file: &OutgoingFile, attachments: &[Zeroizing<Vec<u8>>]) -> usize {
    match (attachments.first(), &file.path) {
        (Some(data), _) if !data.is_empty() => data.len(),
        (_, Some(path)) => std::fs::metadata(path)
            .map_or(0, |file| usize::try_from(file.len()).unwrap_or(usize::MAX)),
        _ => 0,
    }
}

/// The file's decrypted copy in `directory`; for a blocking thread.
fn save_decrypted(
    media: &MediaStore,
    id: i64,
    directory: &str,
    file: &FileToSave,
) -> Result<Value, ApiError> {
    let data = media
        .read(id, MediaPart::File)
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::Internal("file not downloaded".into()))?;
    let directory = std::path::Path::new(directory);
    if !directory.is_dir() {
        return Err(ApiError::Internal("no such directory".into()));
    }
    let name = safe_file_name(file.name.as_deref(), &file.media_type);
    let target = save_target(directory, &name, &data);
    if !target.exists() {
        std::fs::write(&target, data.as_slice()).map_err(|e| ApiError::Internal(e.to_string()))?;
    }
    let openable = target
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(crate::file_types::opens_externally);
    Ok(json!({ "path": target.to_string_lossy(), "openable": openable }))
}

/// How many uploads and downloads are running. A wake-up holds the device up for them: a picture
/// on a slow link would otherwise get the seconds the connection needs, wake-up after wake-up, and
/// could run out of tries overnight.
pub(crate) struct RunningTransfers {
    count: Rc<tokio::sync::watch::Sender<usize>>,
    /// The uploads and downloads under way by message, for the user to stop. A number tells apart
    /// a task and the one that took its place.
    tasks: Rc<RefCell<HashMap<TransferKey, (u64, tokio::task::AbortHandle)>>>,
    started: Cell<u64>,
}

/// What of a message is on its way: its upload, or a part's download.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum TransferKey {
    Upload(i64),
    Download(i64, MediaPart),
}

impl Default for RunningTransfers {
    fn default() -> Self {
        Self {
            count: Rc::new(tokio::sync::watch::channel(0).0),
            tasks: Rc::default(),
            started: Cell::new(0),
        }
    }
}

/// One transfer of those, until it ends, one way or another.
struct RunningTransfer(Rc<tokio::sync::watch::Sender<usize>>);

impl Drop for RunningTransfer {
    fn drop(&mut self) {
        self.0.send_modify(|running| *running -= 1);
    }
}

impl RunningTransfers {
    /// It counts from here on, before its task first runs.
    pub(crate) fn track<T: Future>(&self, transfer: T) -> impl Future<Output = T::Output> + use<T> {
        self.count.send_modify(|running| *running += 1);
        let counted = RunningTransfer(Rc::clone(&self.count));
        async move {
            let _counted = counted;
            transfer.await
        }
    }

    pub(crate) fn none_running(&self) -> bool {
        *self.count.borrow() == 0
    }

    pub(crate) async fn ended(&self) {
        // The sender is ours, it can't be gone.
        let _ = self
            .count
            .subscribe()
            .wait_for(|running| *running == 0)
            .await;
    }

    /// Start a transfer the user can stop.
    fn spawn(&self, key: TransferKey, transfer: impl Future<Output = ()> + 'static) {
        let number = self.started.get() + 1;
        self.started.set(number);
        let tasks = Rc::clone(&self.tasks);
        let task = tokio::task::spawn_local(self.track(async move {
            transfer.await;
            let mut tasks = tasks.borrow_mut();
            if tasks
                .get(&key)
                .is_some_and(|(running, _)| *running == number)
            {
                tasks.remove(&key);
            }
        }));
        self.tasks
            .borrow_mut()
            .insert(key, (number, task.abort_handle()));
    }

    /// Stop it, if it is running. The task goes at its next wait, and with it whatever it holds:
    /// memory, its place in the queue, a connection, a file it was writing.
    fn abort(&self, key: TransferKey) {
        if let Some((_, task)) = self.tasks.borrow_mut().remove(&key) {
            task.abort();
        }
    }
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
        if self.unlocked.get() || !self.store.is_private(&key).unwrap_or(false) {
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
    /// is what is wrong with what was sent.
    pub(crate) async fn send_file_under_pass(
        &mut self,
        file: OutgoingFile,
    ) -> Result<Value, ApiError> {
        let size = file_work_size(&file, &self.attachments);
        let memory = media::reserve_memory_within(size, media::COMMAND_MEMORY_WAIT).await;
        self.send_file_with_memory(file, memory).await
    }

    /// Prepare the file with the attachments, then store it. The share of the memory goes as soon as
    /// the file is on the disk: storing it can lead to more of this, for a list's members.
    async fn send_file_with_memory(
        &mut self,
        file: OutgoingFile,
        memory: media::TransferMemory,
    ) -> Result<Value, ApiError> {
        let identity = file.identity.clone();
        let attachments = std::mem::take(&mut self.attachments);
        let media = Arc::clone(&self.media);
        let prepared = media::off_thread(move || {
            let _memory = memory;
            prepare_file(&media, file, attachments)
        })
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
        self.finish_send_file(&identity, prepared).await
    }

    /// `sendFile`: store the file encrypted, show it right away, upload in the background; the
    /// message is sent once the upload is done.
    pub(crate) async fn send_file(&mut self, file: OutgoingFile) -> Result<Value, ApiError> {
        let (_, conversation) = self.readable_conversation(&file.identity)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        self.send_file_under_pass(file).await
    }

    /// `sendFile` from the UI: the heavy part goes to a blocking thread and the commands behind it
    /// don't wait. The answer comes when it is done ([`Self::finish_file_work`]).
    pub(crate) fn send_file_in_background(
        &mut self,
        file: OutgoingFile,
        under_pass: bool,
    ) -> Result<Value, ApiError> {
        if !under_pass {
            let (_, conversation) = self.readable_conversation(&file.identity)?;
            conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        }
        let identity = file.identity.clone();
        let attachments = std::mem::take(&mut self.attachments);
        let size = file_work_size(&file, &attachments);
        let media = Arc::clone(&self.media);
        // Nothing of the store goes into it: what comes of a chat hidden or gone meanwhile is
        // decided when it is stored.
        self.spawn_file_work(
            size,
            || Ok(()),
            move || FileWork::Send {
                identity,
                prepared: prepare_file(&media, file, attachments),
            },
        );
        Ok(json!({}))
    }

    /// `saveFile` from the UI, likewise.
    pub(crate) fn save_file_in_background(
        &mut self,
        id: i64,
        directory: String,
    ) -> Result<Value, ApiError> {
        let file = self.file_to_save(id)?;
        let size = self.media.stored_size(id, MediaPart::File);
        let conversation = file.conversation.clone();
        let (store, media) = (Rc::clone(&self.store), Arc::clone(&self.media));
        let (checked, check) = (Arc::new(file), Arc::clone(&self.media));
        let to_save = Arc::clone(&checked);
        self.spawn_file_work(
            size,
            move || checked.may_go(&store, &check, id),
            move || FileWork::Save {
                conversation,
                saved: save_decrypted(&media, id, &directory, &to_save),
            },
        );
        Ok(json!({}))
    }

    /// Start `work` on a blocking thread, one at a time: what was asked for first is in the chat
    /// first. It holds a wake-up up, like the transfer it leads to. When its turn has come and `size`
    /// bytes fit in memory, `ready` decides on the core thread whether it still may.
    fn spawn_file_work(
        &mut self,
        size: usize,
        ready: impl FnOnce() -> Result<(), ApiError> + 'static,
        work: impl FnOnce() -> FileWork + Send + 'static,
    ) {
        self.reply_later = true;
        self.file_work_running += 1;
        let request = self.current_request;
        let done = self.file_work_done.clone();
        let turn = Rc::clone(&self.file_work_turn);
        tokio::task::spawn_local(self.transfers.track(async move {
            let _turn = turn.acquire().await;
            let memory = media::reserve_memory_within(size, media::FILE_WORK_MEMORY_WAIT).await;
            let work = match ready() {
                Ok(()) => media::off_thread(move || {
                    let _memory = memory;
                    work()
                })
                .await
                .ok(),
                Err(error) => Some(FileWork::Refused(error)),
            };
            let _ = done.send(FinishedFileWork { request, work });
        }));
    }

    /// The core thread's part of work that was on a blocking thread: the reply to its command.
    pub(crate) async fn finish_file_work(&mut self, finished: FinishedFileWork) -> String {
        self.file_work_running -= 1;
        let outcome = match finished.work {
            Some(FileWork::Refused(error)) => Err(error),
            Some(FileWork::Send { identity, prepared }) => {
                self.finish_send_file(&identity, prepared).await
            }
            // Hidden while it was being saved: the answer would name the file.
            Some(FileWork::Save {
                conversation,
                saved,
            }) => self.check_readable(&conversation).and(saved),
            None => Err(ApiError::Internal("file work failed".into())),
        };
        self.reply(finished.request, None, outcome).await
    }

    /// Store a file that was prepared, whatever happened to its chat since. One hidden meanwhile
    /// (the app locked while the file was on its way) takes it as under a pass: the file was sent
    /// when it could be, and it is the answer that says nothing of the chat. Anything about the
    /// chat reads as the lock.
    async fn finish_send_file(
        &mut self,
        identity: &str,
        prepared: Result<PreparedFile, ApiError>,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.conversation(identity)?;
        let hidden = self.check_readable(&key).is_err();
        let stored = match prepared {
            // A list's members get their copies by reading its own back, not while it is hidden.
            Ok(_) if hidden && matches!(conversation, Conversation::List(_)) => {
                Err(ApiError::Locked)
            }
            Ok(prepared) => self.store_prepared(key, conversation, prepared).await,
            Err(error) => Err(error),
        };
        if !hidden {
            return stored;
        }
        match stored {
            Ok(message) => {
                if let Some(id) = message["id"].as_i64() {
                    self.media.hide(id);
                }
                Ok(json!({}))
            }
            Err(
                error @ (ApiError::MessageTooLong
                | ApiError::FileTooLarge
                | ApiError::VideoKeepsMetadata),
            ) => Err(error),
            Err(error) => {
                tracing::warn!(kind = error.kind(), "file under a write pass refused");
                Err(ApiError::Locked)
            }
        }
    }

    /// Whether a file about to be stored will fit a frame as a message once uploaded, with its
    /// caption escaped for the message's JSON: only here can the user still be told why. Later it
    /// would upload and then fail at sending.
    fn file_fits_a_frame(chat: &str, body: &Value) -> bool {
        let mut uploaded = body.clone();
        uploaded["uploadState"] = json!(STATE_LOCAL);
        // What the upload will add takes the same room whatever its value.
        let blob = libthreema::common::BlobId([0; 16]);
        let keys = FileKeys {
            key: Zeroizing::new([0; 32]),
            blob_id: Some(blob),
            thumbnail_blob_id: (body["thumbnail"] == true).then_some(blob),
        };
        outgoing_file_message(&uploaded, &keys)
            .is_some_and(|file| crate::connection::file_fits_a_frame(chat, file))
    }

    async fn store_prepared(
        &mut self,
        identity: String,
        conversation: Conversation,
        prepared: PreparedFile,
    ) -> Result<Value, ApiError> {
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        let PreparedFile {
            mut body,
            caption,
            file,
            thumbnail,
        } = prepared;
        // A list's copy is not uploaded (each member's copy is).
        body["uploadState"] = json!(if matches!(conversation, Conversation::List(_)) {
            STATE_LOCAL
        } else {
            STATE_LOADING
        });
        if !Self::file_fits_a_frame(&identity, &body) {
            return Err(ApiError::MessageTooLong);
        }
        let message = self.store.insert_outgoing(
            &identity,
            libthreema::common::MessageId::random().0,
            "file",
            &body.to_string(),
        )?;
        let mut key = Zeroizing::new([0_u8; 32]);
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, key.as_mut());
        let stored = (|| {
            file.keep(&self.media, message.id, MediaPart::File)?;
            if let Some(thumbnail) = thumbnail {
                thumbnail.keep(&self.media, message.id, MediaPart::Thumbnail)?;
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
            let mut failed_members = 0;
            for member in self.list_receivers(list)? {
                // What the list got has been made fit to leave; each member gets the same.
                let forwarded = Box::pin(self.copy_message(message.id, &member, false, None)).await;
                if let Err(error) = forwarded {
                    tracing::warn!(%error, "sending a list's file to a member failed");
                    failed_members += 1;
                }
            }
            let mut sent = self.store.message_json(&message);
            // Only how many: why might tell that one of them is a hidden private chat.
            if failed_members > 0 {
                sent["failedMembers"] = failed_members.into();
            }
            return Ok(sent);
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
        let (_, conversation) = self.readable_conversation(&message.conversation)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        if body["uploadState"].as_i64() == Some(STATE_FAILED) {
            self.store.forget_transfer_tries(id, UPLOAD_RETRY)?;
            self.spawn_upload(id);
        }
        Ok(json!({}))
    }

    /// A failed message goes out again, and its file with it: the blob on record may be gone by
    /// now, so it goes up anew (as on Android). Its message waits for that.
    pub(crate) fn upload_again(&mut self, id: i64) -> Result<(), ApiError> {
        self.store.forget_transfer_tries(id, UPLOAD_RETRY)?;
        self.spawn_upload(id);
        Ok(())
    }

    /// `cancelTransfer`: stop a message's running upload or file download. An upload counts as
    /// failed with no try planned, as nothing starts it again but a tap: the message stays, with
    /// what was uploaded so far on record. A download goes back to waiting for a tap, which also
    /// keeps it from coming by itself again.
    pub(crate) fn cancel_transfer(&mut self, id: i64) -> Result<Value, ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|message| message.kind == "file")
            .ok_or_else(|| ApiError::Internal("no such file".into()))?;
        self.check_readable(&message.conversation)?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        let mut changed = None;
        if message.outgoing && body["uploadState"].as_i64() == Some(STATE_LOADING) {
            self.transfers.abort(TransferKey::Upload(id));
            changed = self.store.set_upload_failed(id, false, now_ms())?;
        }
        if body["fileState"].as_i64() == Some(STATE_LOADING) {
            self.transfers
                .abort(TransferKey::Download(id, MediaPart::File));
            changed = self.store.set_download_cancelled(id)?;
        }
        if let Some(changed) = changed {
            let _ = self
                .media_events
                .send(CoreEvent::MessagesChanged(vec![changed]));
        }
        Ok(json!({}))
    }

    /// Online, with a connection or a pause just ended: transfers that failed get another try by
    /// themselves, so a voice message sent on a bad connection does not sit behind its clock until
    /// it is tapped (user report 2026-10-28). Offline there is no use: the next connection comes
    /// here again.
    pub(crate) fn retry_failed_transfers(&mut self) {
        self.transfer_retry_at = None;
        if self.connection_state.get() != "connected" {
            return;
        }
        let now = now_ms();
        let auto = self.auto_download_kinds();
        let limit = self.auto_download_limit();
        match self.store.transfers_to_retry(now, &auto, limit) {
            Ok(retries) => {
                for message in retries.uploads {
                    self.spawn_upload(message);
                }
                for (message, part) in retries.downloads {
                    self.spawn_download(message, part);
                }
                if let Some(at) = retries.next_at {
                    self.look_at_retries_by(at, now);
                }
            }
            Err(error) => tracing::warn!(%error, "listing failed transfers failed"),
        }
    }

    /// The core loop's timer stands for the earliest try planned.
    fn look_at_retries_by(&mut self, at: i64, now: i64) {
        let wait = u64::try_from(at.saturating_sub(now).clamp(0, RETRY_PAUSE_MAX_MS)).unwrap_or(0);
        let at = tokio::time::Instant::now() + std::time::Duration::from_millis(wait);
        self.transfer_retry_at = Some(self.transfer_retry_at.map_or(at, |next| next.min(at)));
    }

    /// Start an upload. It counts as uploading from here on, before its task first runs: the lists
    /// of what is to try again don't name it twice.
    fn spawn_upload(&self, message: i64) {
        if !matches!(self.store.file_keys(message), Ok(Some(_))) {
            return;
        }
        match self.store.set_upload_state(message, STATE_LOADING) {
            Ok(Some(changed)) => {
                let _ = self
                    .media_events
                    .send(CoreEvent::MessagesChanged(vec![changed]));
            }
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "marking an upload failed");
                return;
            }
        }
        self.transfers.spawn(
            TransferKey::Upload(message),
            upload(
                Rc::clone(&self.store),
                self.https.clone(),
                Arc::clone(&self.media),
                self.media_events.clone(),
                message,
            ),
        );
    }

    /// A media task reported progress: an upload may have finished, so send what's pending; one
    /// that failed has its next try to look forward to.
    pub(crate) fn on_media_event(&mut self, event: &CoreEvent) {
        if let CoreEvent::UnknownMembers(identities) = event {
            self.fetch_members(identities.clone());
            return;
        }
        if let CoreEvent::MessagesChanged(messages) = event {
            let now = now_ms();
            for message in messages.iter().filter(|m| m.body.contains("Retry")) {
                let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
                for (state, retry) in [
                    ("uploadState", "uploadRetry"),
                    ("fileState", "fileRetry"),
                    ("thumbnailState", "thumbnailRetry"),
                ] {
                    if body[state].as_i64() == Some(STATE_FAILED)
                        && let Some(at) = body[retry]["at"].as_i64()
                    {
                        self.look_at_retries_by(at, now);
                    }
                }
            }
            // A download's progress is no reason to look for anything to send: only a message that
            // is still to go out is, say one whose upload has just finished.
            if !messages
                .iter()
                .any(|m| m.outgoing && m.state == crate::messages::OutgoingState::Pending as i64)
            {
                return;
            }
        }
        if matches!(event, CoreEvent::ContactsChanged) {
            self.apply_waiting_reflected();
        }
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
            CoreEvent::ContactsChanged => self.apply_waiting_reflected(),
            CoreEvent::RemoveGroupPicture(key) => self.media.remove_picture(key),
            CoreEvent::WebSessionResume(hash) => self.web_resume(hash),
            CoreEvent::DownloadGroupPicture(key) => self.download_group_picture(key.clone()),
            CoreEvent::SendGroupPictures => self.send_group_pictures(),
            CoreEvent::DownloadContactPicture(identity) => {
                self.download_contact_picture(identity.clone());
            }
            CoreEvent::AnnounceGroupCall {
                conversation,
                receivers,
            } => self.announce_running_group_call(conversation, receivers.clone()),
            CoreEvent::GroupCallStart {
                conversation,
                caller,
                start,
            } => self.on_group_call_start(conversation, caller, start),
            CoreEvent::GroupCallRunning { message, call_id } => {
                self.on_group_call_running(message, call_id);
            }
            CoreEvent::CallSignal { peer, kind, json } => self.on_call_signal(peer, *kind, json),
            CoreEvent::SyncPicture { target } => self.apply_synced_picture(target.clone()),
            CoreEvent::GroupDeleted(key) => {
                self.media.remove_picture(key);
                self.chat_removed(key);
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

    /// Start fetching a part. It counts as downloading from here on, before its task first runs:
    /// the lists of what is still to fetch don't name it again, however many messages arrive
    /// in one go. A part with no blob stays as it is.
    fn spawn_download(&self, message: i64, part: MediaPart) {
        let Some(job) = self.download_job(message, part) else {
            return;
        };
        match self.store.set_media_state(message, part, STATE_LOADING) {
            Ok(Some(changed)) => {
                let _ = self
                    .media_events
                    .send(CoreEvent::MessagesChanged(vec![changed]));
            }
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(%error, "marking a media download failed");
                return;
            }
        }
        self.transfers.spawn(
            TransferKey::Download(message, part),
            download(
                Rc::clone(&self.store),
                self.https.clone(),
                Arc::clone(&self.media),
                self.media_events.clone(),
                job,
            ),
        );
    }

    fn download_job(&self, message: i64, part: MediaPart) -> Option<Download> {
        let file = self.store.file_keys(message).ok().flatten()?;
        let blob_id = match part {
            MediaPart::File => file.blob_id,
            MediaPart::Thumbnail => file.thumbnail_blob_id,
        }?;
        let row = self.store.message(message).ok().flatten();
        let seal = match &row {
            Some(row) if part == MediaPart::File => match LegacySeal::of(&self.store, row) {
                Ok(seal) => seal,
                // Failed, not left waiting: a tap shows it, and no pass asks again by itself.
                Err(()) => {
                    tracing::warn!("no key for a deprecated image, not downloading");
                    match self.store.set_media_state(message, part, STATE_FAILED) {
                        Ok(Some(changed)) => {
                            let _ = self
                                .media_events
                                .send(CoreEvent::MessagesChanged(vec![changed]));
                        }
                        Ok(None) => {}
                        Err(error) => tracing::warn!(%error, "marking a media download failed"),
                    }
                    return None;
                }
            },
            _ => None,
        };
        // Android's scopes: our own (reflected) messages' blobs are only on the mirror; others'
        // are public.
        let (outgoing, group) = row.as_ref().map_or((false, false), |m| {
            (m.outgoing, m.conversation.starts_with("g:"))
        });
        let announced_size = row
            .filter(|_| part == MediaPart::File)
            .and_then(|m| serde_json::from_str::<Value>(&m.body).ok())
            .and_then(|body| body["size"].as_u64())
            .map_or(0, |size| usize::try_from(size).unwrap_or(usize::MAX));
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
        Some(Download {
            message,
            part,
            key: file.key,
            seal,
            blob_id,
            scope: if outgoing {
                media::BlobScope::Local
            } else {
                media::BlobScope::Public
            },
            done_scope: done_scope(outgoing, group, media::mirror_active()),
            save_to,
            announced_size,
        })
    }

    /// `downloadFile`: fetch a file (or retry a thumbnail) on request.
    pub(crate) fn download_file(&mut self, id: i64) -> Result<Value, ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|message| message.kind == "file")
            .ok_or_else(|| ApiError::Internal("no such file".into()))?;
        self.check_readable(&message.conversation)?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        for (part, field) in [
            (MediaPart::Thumbnail, "thumbnailState"),
            (MediaPart::File, "fileState"),
        ] {
            let state = body[field].as_i64();
            if matches!(state, Some(STATE_REMOTE | STATE_FAILED))
                && (part == MediaPart::File || body["thumbnail"] == true)
            {
                self.store
                    .forget_transfer_tries(message.id, retry_field(part))?;
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
    /// messages kept. Media only: the files go (thumbnails stay; a download can bring a file back
    /// only while the server still has it, and a received 1:1 file was marked done at its first
    /// download); messages: the messages and their media go. Private chats only while they are
    /// unlocked: changing what is hidden takes the PIN, and this command needs none when the app
    /// lock is off. A period already set for Keep messages applies to them regardless.
    pub(crate) fn clean_up(&mut self, days: u32, messages: bool) -> Result<Value, ApiError> {
        let before = if days == 0 {
            i64::MAX
        } else {
            crate::messages::now_ms() - days_ms(days)
        };
        let hidden = if self.unlocked.get() {
            Vec::new()
        } else {
            self.store.private_chats()?
        };
        let removed = if messages {
            let removed = self.delete_messages_with_media(before, false, &hidden)?;
            // Someone asking for this wants the space, and sees the figure in Settings. Keep
            // messages doesn't do this: its free pages are soon reused, and nobody asked to wait.
            if removed > 0 {
                self.store.shrink_if_worthwhile();
            }
            removed
        } else {
            let ids = self.store.files_before(before, &hidden)?;
            for id in &ids {
                self.media.remove_part(*id, MediaPart::File);
                self.store.set_file_cleaned(*id)?;
            }
            ids.len()
        };
        tracing::info!(removed, messages, "storage cleaned up");
        Ok(json!({ "removed": removed }))
    }

    /// [`Store::delete_messages_before`], and the media of what went.
    fn delete_messages_with_media(
        &self,
        before: i64,
        spare_notes: bool,
        spare_chats: &[String],
    ) -> Result<usize, ApiError> {
        let removed = self
            .store
            .delete_messages_before(before, spare_notes, spare_chats)?;
        let store = Rc::clone(&self.store);
        self.media
            .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
        Ok(removed)
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
        // A stamp from the future was written under a clock set ahead, and says nothing: waiting
        // for that date would silently stop Keep messages until then.
        if last <= now && now - last < 12 * 3_600_000 {
            return false;
        }
        let removed = match self.delete_messages_with_media(now - days_ms(days), true, &[]) {
            Ok(removed) => removed,
            Err(error) => {
                tracing::warn!(kind = error.kind(), "auto-delete failed");
                return false;
            }
        };
        // What waits for its sender's contact holds a message too.
        match self.store.reflected_forget_older(now - days_ms(days)) {
            Ok(0) => {}
            Ok(forgotten) => tracing::info!(forgotten, "auto-delete took waiting messages"),
            Err(error) => tracing::warn!(%error, "auto-delete of waiting messages failed"),
        }
        let _ = self
            .store
            .set_setting(keys::AUTO_DELETE_LAST, &now.to_string());
        tracing::info!(removed, days, "auto-delete ran");
        removed > 0
    }

    /// Maintenance's auto-delete. The browsers hear of it at once: what comes next in maintenance
    /// can wait on the network for long.
    pub(crate) fn expire_messages(&mut self) -> bool {
        let removed = self.auto_delete_scheduled();
        self.web_refresh();
        removed
    }

    /// Remove the media of a message that is gone (deleted, removed).
    pub(crate) fn forget_media(&self, id: i64) {
        self.media.remove(id);
    }

    /// Startup: clean up and resume.
    pub(crate) fn resume_media(&mut self) {
        // First of all: the tasks started below write under the same temporary names.
        self.media.remove_temporary_files();
        // An upload the end of the process cut short is a try that failed, and comes again on the
        // usual schedule: a process that keeps being closed doesn't send it again each time.
        match self.store.unfinished_uploads() {
            Ok(uploads) => {
                let now = now_ms();
                for message in uploads {
                    match self.store.set_upload_failed(message, true, now) {
                        Ok(Some(changed)) => {
                            let _ = self
                                .media_events
                                .send(CoreEvent::MessagesChanged(vec![changed]));
                        }
                        Ok(None) => {}
                        Err(error) => tracing::warn!(%error, "marking an upload failed"),
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "listing uploads failed"),
        }
        if let Err(error) = self.store.reset_loading_media() {
            tracing::warn!(%error, "resetting media states failed");
        }
        let store = Rc::clone(&self.store);
        self.media
            .remove_orphans(|id| store.message_keeps_media(id).unwrap_or(true));
        self.download_pending_media();
    }
}

/// Pictures and videos sent as media get a name that says nothing, like on Android ("to prevent
/// privacy leaks"): phones name them after the moment they were taken. Files keep theirs.
fn generic_media_name(rendering: &str, media_type: &str) -> Option<String> {
    let kind = match media_type.split('/').next() {
        Some("image") => "image",
        Some("video") => "video",
        _ => return None,
    };
    (rendering == "media").then(|| match crate::file_types::extension_for(media_type) {
        Some(extension) => format!("{kind}.{extension}"),
        None => kind.to_owned(),
    })
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
    /// `forwardMessages`, for one: send a text or file message again, to another chat (Android's
    /// "Forward"). Quotes are dropped (the quoted message is not in that chat); files are
    /// uploaded again with a new key, since blobs are deleted once downloaded.
    /// `caption`: what a file goes with instead of its own; empty for none.
    pub(crate) async fn forward_message(
        &mut self,
        id: i64,
        identity: &str,
        caption: Option<String>,
    ) -> Result<Value, ApiError> {
        self.copy_message(id, identity, true, caption).await
    }

    /// A forwarded message, and a list's copy for a member. `clean`: a picture is made fit to leave
    /// first (ADR 0025), as what its file says isn't known, ours included: one may have come from a
    /// linked device. It keeps its size, as on Android. A thumbnail, which comes from the sender
    /// too, is made anew or left behind, as the protocol leaves it optional.
    async fn copy_message(
        &mut self,
        id: i64,
        identity: &str,
        clean: bool,
        caption: Option<String>,
    ) -> Result<Value, ApiError> {
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
                let size = self.media.stored_size(id, MediaPart::File);
                let memory = media::reserve_memory_within(size, media::COMMAND_MEMORY_WAIT).await;
                // That may have taken a moment. The same message, and still to be seen?
                let same = self.store.message(id)?.is_some_and(|now| {
                    now.conversation == message.conversation && now.message_id == message.message_id
                });
                if !same {
                    return Err(ApiError::Internal("no such message".into()));
                }
                self.check_readable(&message.conversation)?;
                let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
                let media = Arc::clone(&self.media);
                let media_type = body["mediaType"]
                    .as_str()
                    .unwrap_or("application/octet-stream")
                    .to_ascii_lowercase();
                let (data, thumbnail) = media::off_thread(move || {
                    Ok::<_, media::MediaError>((
                        media.read(id, MediaPart::File)?,
                        media.read(id, MediaPart::Thumbnail)?,
                    ))
                })
                .await
                .and_then(|read| read)
                .map_err(|e| ApiError::Internal(e.to_string()))?;
                let mut data = data.ok_or(ApiError::FileNotDownloaded)?;
                let mut thumbnail = thumbnail;
                let mut picture = None;
                if clean {
                    let shown_as_media = body["rendering"].as_str().unwrap_or("file") != "file"
                        && !media_type.starts_with("video/")
                        && !media_type.starts_with("audio/");
                    let needs = images::decoding_memory(&data)
                        + thumbnail
                            .as_ref()
                            .map_or(0, |thumbnail| images::decoding_memory(thumbnail));
                    let decoding =
                        media::reserve_memory_within(needs, media::COMMAND_MEMORY_WAIT).await;
                    let claimed = media_type.clone();
                    let (cleaned, sent, made) = media::off_thread(move || {
                        let _memory = decoding;
                        let sent =
                            shown_as_media.then(|| images::sent_picture(&mut data, &claimed, None));
                        let made = thumbnail.and_then(|thumbnail| {
                            images::scaled_jpeg(&thumbnail, images::SENT_THUMBNAIL_MAX, 80)
                        });
                        (data, sent, made)
                    })
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                    data = cleaned;
                    thumbnail = made.map(Zeroizing::new);
                    match sent {
                        Some(Ok(Some(sent))) => {
                            if let Some(made) = sent.thumbnail.clone() {
                                thumbnail = Some(Zeroizing::new(made));
                            }
                            picture = Some(sent);
                        }
                        // By its bytes or its label a picture, and nothing here can clean it.
                        Some(Err(_)) => return Err(ApiError::PictureKeepsMetadata),
                        Some(Ok(None)) if media_type.starts_with("image/") => {
                            return Err(ApiError::PictureKeepsMetadata);
                        }
                        Some(Ok(None)) | None => {}
                    }
                }
                self.attachments = vec![data, thumbnail.unwrap_or_default()];
                let text = |field: &str| body[field].as_str().map(str::to_owned);
                let dimension = |field: &str| {
                    body[field]
                        .as_u64()
                        .and_then(|value| u32::try_from(value).ok())
                };
                let (_, conversation) = self.readable_conversation(identity)?;
                conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
                let result = self
                    .send_file_with_memory(
                        OutgoingFile {
                            identity: identity.to_owned(),
                            path: None,
                            name: text("name"),
                            media_type: picture
                                .as_ref()
                                .map_or(media_type, |picture| picture.media_type.to_owned()),
                            caption: caption.or_else(|| text("caption")),
                            rendering: text("rendering").unwrap_or_else(|| "file".into()),
                            width: picture
                                .as_ref()
                                .and_then(|picture| picture.size)
                                .map(|(width, _)| width)
                                .or_else(|| dimension("width")),
                            height: picture
                                .as_ref()
                                .and_then(|picture| picture.size)
                                .map(|(_, height)| height)
                                .or_else(|| dimension("height")),
                            duration: body["duration"].as_f64(),
                            encode: None,
                            keep_metadata: false,
                        },
                        memory,
                    )
                    .await;
                self.attachments.clear();
                result
            }
            _ => Err(ApiError::NotForwardable),
        }
    }

    /// `saveFile`: write a received or sent file, decrypted, to `directory` (the user's
    /// Downloads); returns the path. This is the only way media leave the encrypted store.
    pub(crate) async fn save_file(
        &mut self,
        id: i64,
        directory: String,
    ) -> Result<Value, ApiError> {
        let size = self.media.stored_size(id, MediaPart::File);
        let memory = media::reserve_memory_within(size, media::COMMAND_MEMORY_WAIT).await;
        let file = self.file_to_save(id)?;
        let media = Arc::clone(&self.media);
        media::off_thread(move || {
            let _memory = memory;
            save_decrypted(&media, id, &directory, &file)
        })
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
    }

    /// The chat and what the file is called, if it may leave.
    fn file_to_save(&self, id: i64) -> Result<FileToSave, ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|message| message.kind == "file")
            .ok_or_else(|| ApiError::Internal("no such file".into()))?;
        self.check_readable(&message.conversation)?;
        let body: Value = serde_json::from_str(&message.body).unwrap_or_default();
        Ok(FileToSave {
            name: body["name"].as_str().map(str::to_owned),
            media_type: body["mediaType"].as_str().unwrap_or("").to_owned(),
            conversation: message.conversation,
            message_id: message.message_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start's clean-up finds file messages by their index; a chat's own go by the chat.
    #[test]
    fn file_messages_are_found_without_reading_all_messages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let plan = |sql: &str| -> String {
            let mut statement = store
                .conn
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
                .unwrap();
            let rows = statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap();
            rows.map(Result::unwrap).collect::<Vec<_>>().join(" ")
        };
        // `reset_loading_media`; the others have narrower indexes of their own.
        let reset = "UPDATE messages SET body = json_set(body, '$.fileState', 0)
             WHERE kind = 'file' AND json_extract(body, '$.fileState') = 1";
        assert!(plan(reset).contains("messages_files"), "{}", plan(reset));
        let in_a_chat = "SELECT id FROM messages WHERE conversation = 'x' AND kind = 'file'";
        assert!(
            !plan(in_a_chat).contains("messages_files"),
            "{}",
            plan(in_a_chat)
        );
    }

    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &crate::app::StartConfig {
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

    #[test]
    fn a_failed_upload_is_not_retried_in_a_chat_that_sends_nothing() {
        use libthreema::protobuf::d2d_sync::contact::ActivityState;
        let (_dir, mut app) = app();
        let mut contact = crate::contacts::test_contact("ABCD1234");
        app.store.upsert_contact(&contact).unwrap();
        let body = json!({ "uploadState": STATE_FAILED }).to_string();
        let id = app
            .store
            .insert_outgoing("ABCD1234", 1, "file", &body)
            .unwrap()
            .id;

        let mut settings = app.app_settings().unwrap();
        settings.blocked_contacts = vec!["ABCD1234".into()];
        app.save_app_settings(&settings).unwrap();
        assert_eq!(app.retry_upload(id).unwrap_err().kind(), "contactBlocked");

        settings.blocked_contacts.clear();
        app.save_app_settings(&settings).unwrap();
        contact.activity_state = ActivityState::Invalid;
        app.store.upsert_contact(&contact).unwrap();
        assert_eq!(app.retry_upload(id).unwrap_err().kind(), "contactRevoked");
    }

    /// A received file message whose parts wait on the server, with this blob key.
    fn received_file(store: &Store, message_id: u64, key: u8) -> i64 {
        let file = FileMessage {
            rendering_type: FileRenderingType::Media,
            key: [key; 32],
            blob_id: libthreema::common::BlobId([key; 16]),
            media_type: "image/jpeg".into(),
            name: Some("a.jpg".into()),
            size: 1000,
            thumbnail_blob_id: Some(libthreema::common::BlobId([key + 1; 16])),
            thumbnail_media_type: None,
            caption: None,
            correlation_id: None,
            metadata: libthreema::model::message::FileMetadata::default(),
        };
        let stored = store
            .insert_incoming(
                "ABCD1234",
                message_id,
                "file",
                &file_body(&file).to_string(),
                1,
            )
            .unwrap()
            .unwrap();
        store.set_file_keys(stored.id, &file_keys(&file)).unwrap();
        stored.id
    }

    fn state(store: &Store, message: i64, field: &str) -> Option<i64> {
        let body: Value = serde_json::from_str(&store.message(message).unwrap()?.body).ok()?;
        body[field].as_i64()
    }

    /// Several file messages arriving in one go list each other's parts again and again: each part
    /// is fetched once, and it counts as downloading before its task has run.
    #[tokio::test(flavor = "current_thread")]
    async fn a_burst_of_files_starts_each_download_once() {
        let (_dir, mut app) = app();
        let first = received_file(&app.store, 1, 1);
        let second = received_file(&app.store, 2, 3);
        let images = vec!["image".to_owned()];
        let mut rx = app.take_media_events().unwrap();
        let local = tokio::task::LocalSet::new();
        // Ends without yielding: no task runs, so nothing goes to the network.
        local
            .run_until(async {
                app.download_pending_media();
                assert_eq!(state(&app.store, first, "fileState"), Some(STATE_LOADING));
                assert_eq!(
                    state(&app.store, second, "thumbnailState"),
                    Some(STATE_LOADING)
                );
                assert!(app.store.media_to_download(&images, 0).unwrap().is_empty());
                let third = received_file(&app.store, 3, 5);
                assert_eq!(
                    app.store.media_to_download(&images, 0).unwrap(),
                    vec![(third, MediaPart::Thumbnail), (third, MediaPart::File)]
                );
                // A tap on what is downloading already starts nothing either.
                while rx.try_recv().is_ok() {}
                app.download_file(first).unwrap();
                assert!(rx.try_recv().is_err());
            })
            .await;
    }

    /// A part with no blob to fetch doesn't claim to be downloading, or it would stay so.
    #[tokio::test(flavor = "current_thread")]
    async fn a_part_with_no_blob_keeps_its_state() {
        let (_dir, mut app) = app();
        let body = json!({"mediaType": "image/jpeg", "fileState": STATE_FAILED,
                          "thumbnail": false, "thumbnailState": STATE_FAILED});
        let message = app
            .store
            .insert_incoming("ABCD1234", 1, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async { app.download_file(message.id).unwrap() })
            .await;
        assert_eq!(
            state(&app.store, message.id, "fileState"),
            Some(STATE_FAILED)
        );
    }

    fn job(store: &Store, message: i64, part: MediaPart) -> Download {
        let keys = store.file_keys(message).unwrap().unwrap();
        Download {
            message,
            part,
            blob_id: match part {
                MediaPart::File => keys.blob_id.unwrap(),
                MediaPart::Thumbnail => keys.thumbnail_blob_id.unwrap(),
            },
            key: keys.key,
            seal: None,
            scope: media::BlobScope::Public,
            done_scope: None,
            save_to: None,
            announced_size: 0,
        }
    }

    /// [`Download::keep`], from a test without a runtime.
    fn kept(
        job: &Download,
        app: &App,
        events: &mpsc::UnboundedSender<CoreEvent>,
        fetched: Result<Vec<u8>, media::MediaError>,
    ) -> bool {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(job.keep(&app.store, &app.media, events, fetched))
    }

    /// Runs `command` in a set of its own: the upload it spawns last goes with the set, without
    /// having run, so nothing goes to the network.
    async fn in_own_set<F: Future>(command: F) -> F::Output {
        tokio::task::LocalSet::new().run_until(command).await
    }

    fn auto_save(store: &Store, message: i64, data: &[u8], pictures: &std::path::Path) {
        if let Some(gallery) = GallerySave::of(store, message, pictures) {
            gallery.write(data);
        }
    }

    /// The core thread keeps running while a blob is decrypted and stored: with the connection on
    /// the same thread, a large file would otherwise leave its echoes unanswered.
    #[tokio::test(flavor = "current_thread")]
    async fn storing_a_download_leaves_the_core_thread_free() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = received_file(&app.store, 1, 1);
        let download = job(&app.store, id, MediaPart::File);
        let blob = media::encrypt_blob(&[1; 32], MediaPart::File, &[7; 1024 * 1024]);
        let turns = Rc::new(std::cell::Cell::new(0));
        let counted = Rc::clone(&turns);
        tokio::task::LocalSet::new()
            .run_until(async {
                let others = tokio::task::spawn_local(async move {
                    loop {
                        counted.set(counted.get() + 1);
                        tokio::task::yield_now().await;
                    }
                });
                assert!(
                    download
                        .keep(&app.store, &app.media, &events, Ok(blob))
                        .await
                );
                others.abort();
            })
            .await;
        assert!(turns.get() > 0, "others got no turn");
        assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOCAL));
        assert_eq!(
            app.media.read(id, MediaPart::File).unwrap().unwrap().len(),
            1024 * 1024
        );
    }

    /// The same for what goes the other way.
    #[tokio::test(flavor = "current_thread")]
    async fn reading_and_encrypting_an_upload_leaves_the_core_thread_free() {
        let (_dir, app) = app();
        let id = outgoing_file(&app.store, 1, 1);
        app.media
            .write(id, MediaPart::File, &[7; 1024 * 1024])
            .unwrap();
        let turns = Rc::new(std::cell::Cell::new(0));
        let counted = Rc::clone(&turns);
        let turns_at_send = std::cell::Cell::new(None);
        let send = async |blob: Vec<u8>| {
            turns_at_send.set(Some((turns.get(), blob.len())));
            Ok(libthreema::common::BlobId([1; 16]))
        };
        let mut keys = app.store.file_keys(id).unwrap().unwrap();
        tokio::task::LocalSet::new()
            .run_until(async {
                let others = tokio::task::spawn_local(async move {
                    loop {
                        counted.set(counted.get() + 1);
                        tokio::task::yield_now().await;
                    }
                });
                upload_parts(&app.store, &app.media, id, &mut keys, TO_CONTACT, &send)
                    .await
                    .unwrap();
                others.abort();
            })
            .await;
        let (turns, size) = turns_at_send.get().unwrap();
        assert!(turns > 0, "others got no turn");
        assert_eq!(size, 1024 * 1024 + 16);
    }

    /// Deleted while a blocking thread was busy with it, and the next message got its row id.
    #[test]
    fn a_file_sealed_for_a_message_that_went_stays_out_of_the_next_one() {
        let (dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = received_file(&app.store, 1, 1);
        let download = job(&app.store, id, MediaPart::File);
        let sealed = app.media.seal(Zeroizing::new(b"old".to_vec())).unwrap();
        app.store.remove_message(id).unwrap();
        let next = received_file(&app.store, 2, 3);
        assert_eq!(next, id);

        assert!(!download.finish(&app.store, &app.media, &events, Ok(sealed)));
        assert!(!app.media.has(next, MediaPart::File));
        let left = std::fs::read_dir(dir.path().join("media")).unwrap().count();
        assert_eq!(left, 0, "nothing stayed behind");
    }

    #[test]
    fn a_download_sets_memory_aside_by_what_its_sender_says_within_the_limits() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        let mut file = job(&app.store, id, MediaPart::File);
        assert_eq!(
            file.first_limit(),
            media::MAX_PICTURE_DOWNLOAD,
            "says nothing"
        );
        file.announced_size = 50 * 1024 * 1024;
        assert_eq!(file.first_limit(), 50 * 1024 * 1024 + 16);
        file.announced_size = usize::MAX;
        assert_eq!(file.first_limit(), media::MAX_FILE_DOWNLOAD);
        let mut thumbnail = job(&app.store, id, MediaPart::Thumbnail);
        thumbnail.announced_size = 50 * 1024 * 1024;
        assert_eq!(thumbnail.first_limit(), media::MAX_PICTURE_DOWNLOAD);
    }

    #[test]
    fn a_download_knows_what_its_sender_said_of_the_file() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        let file_size = serde_json::from_str::<Value>(
            &app.store.message(id).unwrap().unwrap().body,
        )
        .unwrap()["size"]
            .as_u64()
            .unwrap();
        let of = |part| app.download_job(id, part).unwrap().announced_size;
        assert_eq!(of(MediaPart::File) as u64, file_size);
        assert_eq!(of(MediaPart::Thumbnail), 0);
    }

    /// A deprecated image comes down like any file, but its blob opens with what sender and
    /// receiver share, under the message's own nonce. The nonce stays away from the UI.
    #[test]
    fn a_deprecated_image_opens_with_the_key_between_sender_and_receiver() {
        use libthreema::common::{BlobId, keys::ClientKey};
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let sender = ClientKey::from([8; 32]);
        let mut contact = crate::contacts::test_contact("ABCD1234");
        contact.public_key = sender.public_key();
        app.store.upsert_contact(&contact).unwrap();

        let nonce = [0x4e; 24];
        let file = FileMessage {
            rendering_type: FileRenderingType::Media,
            key: [0; 32],
            blob_id: BlobId([1; 16]),
            media_type: "image/jpeg".into(),
            name: None,
            size: 7,
            thumbnail_blob_id: None,
            thumbnail_media_type: None,
            caption: None,
            correlation_id: None,
            metadata: libthreema::model::message::FileMetadata {
                legacy_nonce: Some(nonce),
                ..Default::default()
            },
        };
        let stored = app
            .store
            .insert_incoming("ABCD1234", 5, "file", &file_body(&file).to_string(), 1)
            .unwrap()
            .unwrap();
        app.store
            .set_file_keys(stored.id, &file_keys(&file))
            .unwrap();
        assert_eq!(
            stored.to_json()["body"].get(LEGACY_NONCE),
            None,
            "the UI has no use for it"
        );

        let own_public = ClientKey::from([7; 32]).public_key();
        let shared = sender.derive_shared_secret(&own_public).unwrap();
        let blob = {
            use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305, aead::Aead as _};
            XSalsa20Poly1305::new((&*shared).into())
                .encrypt((&nonce).into(), b"picture".as_slice())
                .unwrap()
        };

        let download = app.download_job(stored.id, MediaPart::File).unwrap();
        assert!(kept(&download, &app, &events, Ok(blob)));
        assert_eq!(
            app.media
                .read(stored.id, MediaPart::File)
                .unwrap()
                .unwrap()
                .as_slice(),
            b"picture"
        );

        // A normal file has no seal, and one we can't open isn't fetched only to fail.
        let usual = received_file(&app.store, 6, 1);
        assert!(
            app.download_job(usual, MediaPart::File)
                .unwrap()
                .seal
                .is_none()
        );
        app.store.delete_contact("ABCD1234").unwrap();
        let lost = app
            .store
            .insert_incoming("ABCD1234", 7, "file", &file_body(&file).to_string(), 1)
            .unwrap()
            .unwrap();
        app.store.set_file_keys(lost.id, &file_keys(&file)).unwrap();
        assert!(app.download_job(lost.id, MediaPart::File).is_none());
        assert_eq!(state(&app.store, lost.id, "fileState"), Some(STATE_FAILED));
    }

    #[test]
    fn a_finished_download_is_kept_and_a_late_failure_harms_no_file() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = received_file(&app.store, 1, 1);
        let download = job(&app.store, id, MediaPart::File);
        let blob = media::encrypt_blob(&[1; 32], MediaPart::File, b"picture");

        assert!(kept(&download, &app, &events, Ok(blob)));
        assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOCAL));
        assert_eq!(
            app.media
                .read(id, MediaPart::File)
                .unwrap()
                .unwrap()
                .as_slice(),
            b"picture"
        );

        // A second fetch of the same blob, which the server has dropped since.
        let gone = Err(media::MediaError::Status(404));
        assert!(!kept(&download, &app, &events, gone));
        assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOCAL));

        // Nothing here and nothing fetched: failed.
        let other = job(&app.store, id, MediaPart::Thumbnail);
        let gone = Err(media::MediaError::Status(404));
        assert!(!kept(&other, &app, &events, gone));
        assert_eq!(state(&app.store, id, "thumbnailState"), Some(STATE_FAILED));
    }

    /// Deleted for everyone while it was being fetched: "Message deleted" in the chat, and none of
    /// the content on the device.
    #[test]
    fn a_download_finished_after_the_deletion_stores_nothing() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = received_file(&app.store, 1, 1);
        let download = job(&app.store, id, MediaPart::File);
        let blob = media::encrypt_blob(&[1; 32], MediaPart::File, b"picture");
        app.store.mark_deleted(id, 5).unwrap();

        assert!(!kept(&download, &app, &events, Ok(blob)));
        assert!(!app.media.has(id, MediaPart::File));
    }

    /// A row id that came back: what was fetched for the old message stays out of the new one,
    /// and so does its failure.
    #[test]
    fn a_late_download_leaves_the_next_message_with_that_id_alone() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = received_file(&app.store, 1, 1);
        let download = job(&app.store, id, MediaPart::File);
        let blob = media::encrypt_blob(&[1; 32], MediaPart::File, b"old picture");
        app.store.remove_message(id).unwrap();
        let next = received_file(&app.store, 2, 3);
        assert_eq!(next, id, "SQLite gives the newest row's id out again");

        assert!(!kept(&download, &app, &events, Ok(blob)));
        assert!(!app.media.has(next, MediaPart::File));
        let failed = Err(media::MediaError::Status(404));
        assert!(!kept(&download, &app, &events, failed));
        assert_eq!(state(&app.store, next, "fileState"), Some(STATE_REMOTE));
    }

    /// The same sender using the same key again doesn't make it the same file.
    #[test]
    fn a_late_download_goes_by_the_blob_as_well() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        let download = job(&app.store, id, MediaPart::File);
        assert!(download.wanted(&app.store));
        let mut keys = app.store.file_keys(id).unwrap().unwrap();
        keys.blob_id = Some(libthreema::common::BlobId([9; 16]));
        app.store.set_file_keys(id, &keys).unwrap();
        assert!(!download.wanted(&app.store));
    }

    fn outgoing_file(store: &Store, message_id: u64, key: u8) -> i64 {
        let body = json!({"mediaType": "video/mp4", "size": 1, "uploadState": STATE_LOADING});
        let message = store
            .insert_outgoing("ABCD1234", message_id, "file", &body.to_string())
            .unwrap();
        store
            .set_file_keys(
                message.id,
                &FileKeys {
                    key: Zeroizing::new([key; 32]),
                    blob_id: None,
                    thumbnail_blob_id: None,
                },
            )
            .unwrap();
        message.id
    }

    /// The wrong video deleted during its upload, and the right one sent, under the same row id:
    /// the old upload's blob must not go out as the new message.
    #[test]
    fn an_upload_of_a_deleted_file_leaves_the_next_file_alone() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let wrong = outgoing_file(&app.store, 1, 1);
        let mut uploaded = app.store.file_keys(wrong).unwrap().unwrap();
        uploaded.blob_id = Some(libthreema::common::BlobId([7; 16]));
        app.store.remove_message(wrong).unwrap();
        let right = outgoing_file(&app.store, 2, 2);
        assert_eq!(right, wrong, "SQLite gives the newest row's id out again");

        finish_upload(&app.store, &events, wrong, &uploaded, Ok(()));
        let keys = app.store.file_keys(right).unwrap().unwrap();
        assert_eq!(*keys.key, [2; 32]);
        assert!(keys.blob_id.is_none());
        assert_eq!(
            state(&app.store, right, "uploadState"),
            Some(STATE_LOADING),
            "still its own upload's business"
        );

        // Nor does its failure hold the new one back.
        let failed = Err(media::MediaError::Status(500));
        finish_upload(&app.store, &events, wrong, &uploaded, failed);
        assert_eq!(state(&app.store, right, "uploadState"), Some(STATE_LOADING));
    }

    #[test]
    fn a_finished_upload_is_recorded() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = outgoing_file(&app.store, 1, 1);
        let mut uploaded = app.store.file_keys(id).unwrap().unwrap();
        uploaded.blob_id = Some(libthreema::common::BlobId([7; 16]));
        finish_upload(&app.store, &events, id, &uploaded, Ok(()));
        let keys = app.store.file_keys(id).unwrap().unwrap();
        assert_eq!(keys.blob_id.unwrap().0, [7; 16]);
        assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOCAL));

        let failed = Err(media::MediaError::Status(500));
        finish_upload(&app.store, &events, id, &uploaded, failed);
        assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
    }

    /// A failure of the network gets another try; a file that is gone can't get better, so the
    /// message shows as failed and stops being retried.
    #[test]
    fn an_upload_without_its_file_fails_for_good() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = outgoing_file(&app.store, 1, 1);
        let keys = app.store.file_keys(id).unwrap().unwrap();
        let state_of = || app.store.message(id).unwrap().unwrap().state;
        let later = now_ms() + RETRY_PAUSE_MAX_MS;

        let network = Err(media::MediaError::Network("timeout".into()));
        finish_upload(&app.store, &events, id, &keys, network);
        assert_eq!(
            app.store.transfers_to_retry(later, &[], 0).unwrap().uploads,
            vec![id]
        );
        assert_eq!(state_of(), crate::messages::OutgoingState::Pending as i64);

        let gone = Err(media::MediaError::FileGone);
        finish_upload(&app.store, &events, id, &keys, gone);
        assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
        assert_eq!(state_of(), crate::messages::OutgoingState::Failed as i64);
        assert!(
            app.store
                .transfers_to_retry(later, &[], 0)
                .unwrap()
                .uploads
                .is_empty()
        );
        assert!(app.store.pending_outgoing().unwrap().is_empty());
    }

    /// Each failure in a row waits longer, and the tries run out.
    #[test]
    fn a_failing_upload_pauses_longer_and_stops() {
        let (_dir, app) = app();
        let id = outgoing_file(&app.store, 1, 1);
        let mut now = 1_000_000;
        for minutes in RETRY_PAUSES_MINUTES {
            app.store.set_upload_failed(id, true, now).unwrap();
            let retries = app.store.transfers_to_retry(now, &[], 0).unwrap();
            assert!(retries.uploads.is_empty(), "not before the pause is over");
            assert_eq!(retries.next_at, Some(now + minutes * MINUTE_MS));
            now += minutes * MINUTE_MS;
            assert_eq!(
                app.store.transfers_to_retry(now, &[], 0).unwrap().uploads,
                vec![id]
            );
        }
        app.store.set_upload_failed(id, true, now).unwrap();
        assert_eq!(
            app.store
                .transfers_to_retry(now + 365 * RETRY_PAUSE_MAX_MS, &[], 0)
                .unwrap(),
            TransferRetries::default(),
            "out of tries"
        );

        // A tap starts them over.
        app.store.forget_transfer_tries(id, UPLOAD_RETRY).unwrap();
        app.store.set_upload_failed(id, true, now).unwrap();
        assert_eq!(
            app.store.transfers_to_retry(now, &[], 0).unwrap().next_at,
            Some(now + MINUTE_MS)
        );

        // And what got through leaves no note.
        app.store.set_upload_state(id, STATE_LOCAL).unwrap();
        let body = app.store.message(id).unwrap().unwrap().body;
        assert!(!body.contains("uploadRetry"), "{body}");
    }

    /// What the server refused doesn't get better by itself, its "not now" does.
    #[test]
    fn a_refused_upload_waits_for_a_tap() {
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let later = now_ms() + 2 * MINUTE_MS;
        let keys = |id| app.store.file_keys(id).unwrap().unwrap();

        let too_large = outgoing_file(&app.store, 1, 1);
        let refused = Err(media::MediaError::Status(413));
        finish_upload(&app.store, &events, too_large, &keys(too_large), refused);
        assert_eq!(
            state(&app.store, too_large, "uploadState"),
            Some(STATE_FAILED)
        );
        assert_eq!(
            app.store.message(too_large).unwrap().unwrap().state,
            crate::messages::OutgoingState::Pending as i64,
            "a tap can still send it"
        );

        let busy = outgoing_file(&app.store, 2, 2);
        let not_now = Err(media::MediaError::Status(429));
        finish_upload(&app.store, &events, busy, &keys(busy), not_now);

        assert_eq!(
            app.store.transfers_to_retry(later, &[], 0).unwrap().uploads,
            vec![busy]
        );
    }

    /// An upload that failed before tries were counted isn't forgotten.
    #[test]
    fn an_upload_failed_before_tries_were_counted_is_due() {
        let (_dir, app) = app();
        let id = outgoing_file(&app.store, 1, 1);
        app.store.set_upload_state(id, STATE_FAILED).unwrap();
        assert_eq!(
            app.store.transfers_to_retry(0, &[], 0).unwrap().uploads,
            vec![id]
        );
    }

    /// A clock that went back doesn't hold a try up until it has caught up.
    #[test]
    fn a_try_planned_by_a_clock_since_gone_back_is_due() {
        let (_dir, app) = app();
        let id = outgoing_file(&app.store, 1, 1);
        let ahead = 1_000 * 24 * 60 * MINUTE_MS;
        app.store.set_upload_failed(id, true, ahead).unwrap();
        assert_eq!(
            app.store.transfers_to_retry(0, &[], 0).unwrap().uploads,
            vec![id]
        );
    }

    /// Starting one marks it at once: a second look before its task ran doesn't name it again.
    #[tokio::test(flavor = "current_thread")]
    async fn a_retried_upload_starts_once() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                app.connection_state.set("connected");
                let id = outgoing_file(&app.store, 1, 1);
                app.store.set_upload_state(id, STATE_FAILED).unwrap();
                app.retry_failed_transfers();
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOADING));
                assert!(
                    app.store
                        .transfers_to_retry(now_ms(), &[], 0)
                        .unwrap()
                        .uploads
                        .is_empty()
                );
            })
            .await;
    }

    /// A wake-up looks too: the loop's timer stands still while the phone sleeps, the clock doesn't.
    #[tokio::test(flavor = "current_thread")]
    async fn a_wake_up_starts_what_came_due_during_sleep() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                app.connection_state.set("connected");
                let id = outgoing_file(&app.store, 1, 1);
                app.store.set_upload_failed(id, true, 0).unwrap();
                app.wake_connection();
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOADING));
            })
            .await;
    }

    /// Offline nothing starts, and no timer runs for it: the next connection looks.
    #[tokio::test(flavor = "current_thread")]
    async fn offline_nothing_tries_again() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                app.connection_state.set("disconnected");
                let id = outgoing_file(&app.store, 1, 1);
                app.store.set_upload_failed(id, true, 0).unwrap();
                app.retry_failed_transfers();
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
                assert!(app.transfer_retry_at.is_none());
            })
            .await;
    }

    /// A photo coming in changes its message several times; none of that sends anything. An upload
    /// that has finished does. So does what isn't about a message's state.
    #[tokio::test(flavor = "current_thread")]
    async fn only_a_change_to_what_is_still_to_go_out_sends_pending_messages() {
        let (_dir, mut app) = app();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        app.connection.replace(Some(tx));
        let mut asked = |app: &mut App, event: CoreEvent| {
            app.on_media_event(&event);
            let mut count = 0;
            while let Ok(message) = rx.try_recv() {
                assert!(matches!(
                    message,
                    crate::connection::ToConnection::SendPending
                ));
                count += 1;
            }
            count
        };

        let incoming = app
            .store
            .insert_incoming("00000BOB", 1, "file", "{}", 1)
            .unwrap()
            .unwrap();
        let event = CoreEvent::MessagesChanged(vec![incoming]);
        assert_eq!(asked(&mut app, event), 0);

        let id = outgoing_file(&app.store, 2, 1);
        let pending = app.store.message(id).unwrap().unwrap();
        let event = CoreEvent::MessagesChanged(vec![pending]);
        assert_eq!(asked(&mut app, event), 1);

        // Our own file, reflected or sent long ago, downloaded again.
        app.store
            .raise_outgoing_state("ABCD1234", &[2], crate::messages::OutgoingState::Sent)
            .unwrap();
        let sent = app.store.message(id).unwrap().unwrap();
        let event = CoreEvent::MessagesChanged(vec![sent]);
        assert_eq!(asked(&mut app, event), 0);

        assert_eq!(asked(&mut app, CoreEvent::GroupsChanged), 1);
    }

    /// A pause in progress has the loop's timer set for its end, from the event of the failure and
    /// from a look at the list alike.
    #[tokio::test(flavor = "current_thread")]
    async fn the_loop_wakes_for_the_next_try() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                let id = outgoing_file(&app.store, 1, 1);
                let failed = app
                    .store
                    .set_upload_failed(id, true, now_ms())
                    .unwrap()
                    .unwrap();
                app.on_media_event(&CoreEvent::MessagesChanged(vec![failed]));
                let wait = app.transfer_retry_at.unwrap() - tokio::time::Instant::now();
                assert!(wait <= std::time::Duration::from_secs(60));
                assert!(wait > std::time::Duration::from_secs(30));

                app.connection_state.set("connected");
                app.transfer_retry_at = None;
                app.retry_failed_transfers();
                assert!(app.transfer_retry_at.is_some());
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
            })
            .await;
    }

    /// An upload cut short by the end of the process counts as a try: each start leaves it to the
    /// usual schedule, and the tries run out as for any failure.
    #[tokio::test(flavor = "current_thread")]
    async fn an_upload_cut_short_counts_as_a_try() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                let id = outgoing_file(&app.store, 1, 1);
                for tries in 1..=RETRY_PAUSES_MINUTES.len() {
                    app.store.set_upload_state(id, STATE_LOADING).unwrap();
                    app.resume_media();
                    assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
                    let retries = app.store.transfers_to_retry(now_ms(), &[], 0).unwrap();
                    assert!(retries.uploads.is_empty(), "not at the start itself");
                    let waits = RETRY_PAUSES_MINUTES[tries - 1] * MINUTE_MS;
                    assert!(retries.next_at.unwrap() <= now_ms() + waits);
                }
                app.store.set_upload_state(id, STATE_LOADING).unwrap();
                app.resume_media();
                assert_eq!(
                    app.store
                        .transfers_to_retry(now_ms() + 365 * RETRY_PAUSE_MAX_MS, &[], 0)
                        .unwrap(),
                    TransferRetries::default(),
                    "out of tries"
                );
            })
            .await;
    }

    const TO_CONTACT: UploadTarget = UploadTarget {
        mirror: false,
        scope: media::BlobScope::Public,
        persist: false,
    };

    /// The thumbnail failing doesn't send the file a second time, and the blob that did go up ends
    /// up in the message.
    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_thumbnail_keeps_the_uploaded_file() {
        use std::cell::{Cell, RefCell};
        let (_dir, app) = app();
        let (events, _rx) = mpsc::unbounded_channel();
        let id = outgoing_file(&app.store, 1, 1);
        app.media.write(id, MediaPart::File, b"file").unwrap();
        app.media.write(id, MediaPart::Thumbnail, b"thumb").unwrap();
        let key = *app.store.file_keys(id).unwrap().unwrap().key;
        let file = media::encrypt_blob(&key, MediaPart::File, b"file");
        let sent = RefCell::new(Vec::new());
        let thumbnail_works = Cell::new(false);
        let send = async |blob: Vec<u8>| {
            sent.borrow_mut().push(blob.clone());
            if blob == file {
                Ok(libthreema::common::BlobId([1; 16]))
            } else if thumbnail_works.get() {
                Ok(libthreema::common::BlobId([2; 16]))
            } else {
                Err(media::MediaError::Network("timeout".into()))
            }
        };

        let mut keys = app.store.file_keys(id).unwrap().unwrap();
        let result = upload_parts(&app.store, &app.media, id, &mut keys, TO_CONTACT, &send).await;
        finish_upload(&app.store, &events, id, &keys, result);
        assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
        assert_eq!(sent.borrow().len(), 2);
        let stored = app.store.file_keys(id).unwrap().unwrap();
        assert_eq!(stored.blob_id.unwrap().0, [1; 16]);

        thumbnail_works.set(true);
        let mut keys = app.store.file_keys(id).unwrap().unwrap();
        let result = upload_parts(&app.store, &app.media, id, &mut keys, TO_CONTACT, &send).await;
        finish_upload(&app.store, &events, id, &keys, result);
        assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOCAL));
        assert_eq!(sent.borrow().len(), 3, "the thumbnail alone");
        assert_ne!(sent.borrow()[2], file);
        let stored = app.store.file_keys(id).unwrap().unwrap();
        assert_eq!(stored.blob_id.unwrap().0, [1; 16]);
        assert_eq!(stored.thumbnail_blob_id.unwrap().0, [2; 16]);
        let body = app.store.message(id).unwrap().unwrap().body;
        assert!(!body.contains("uploadedFile"), "{body}");
    }

    /// A file part that went up is sent again if it would now go another way: a notes group that
    /// gained a member needs a blob for the others to fetch, which stays.
    #[tokio::test(flavor = "current_thread")]
    async fn a_file_part_kept_for_another_way_goes_up_again() {
        use std::cell::Cell;
        let (_dir, app) = app();
        let id = outgoing_file(&app.store, 1, 1);
        app.media.write(id, MediaPart::File, b"file").unwrap();
        let sent = Cell::new(0);
        let send = async |_: Vec<u8>| {
            sent.set(sent.get() + 1);
            Ok(libthreema::common::BlobId([1; 16]))
        };
        let notes = UploadTarget {
            mirror: false,
            scope: media::BlobScope::Local,
            persist: false,
        };
        let group = UploadTarget {
            mirror: false,
            scope: media::BlobScope::Public,
            persist: true,
        };
        let mut keys = app.store.file_keys(id).unwrap().unwrap();
        upload_parts(&app.store, &app.media, id, &mut keys, notes, &send)
            .await
            .unwrap();
        assert_eq!(sent.get(), 1);
        upload_parts(&app.store, &app.media, id, &mut keys, notes, &send)
            .await
            .unwrap();
        assert_eq!(sent.get(), 1, "the same way: no need");
        upload_parts(&app.store, &app.media, id, &mut keys, group, &send)
            .await
            .unwrap();
        assert_eq!(sent.get(), 2);
    }

    /// An uploaded file part is good for a day, and for the way it went.
    #[test]
    fn how_long_an_uploaded_file_is_good() {
        let body = json!({"uploadedFile": {
            "at": 1000, "mirror": false, "scope": "public", "persist": false
        }});
        let other = |mirror, scope, persist| UploadTarget {
            mirror,
            scope,
            persist,
        };
        let (public, local) = (media::BlobScope::Public, media::BlobScope::Local);
        assert!(uploaded_file_good(&body, 1000, TO_CONTACT));
        let last = 1000 + UPLOADED_FILE_KEPT_MS - 1;
        assert!(uploaded_file_good(&body, last, TO_CONTACT));
        assert!(!uploaded_file_good(&body, last + 1, TO_CONTACT));
        assert!(
            !uploaded_file_good(&body, 999, TO_CONTACT),
            "a clock gone back"
        );
        let mirror = other(true, public, false);
        assert!(
            !uploaded_file_good(&body, 1000, mirror),
            "another server now"
        );
        let scope = other(false, local, false);
        assert!(!uploaded_file_good(&body, 1000, scope), "another scope now");
        let persist = other(false, public, true);
        assert!(
            !uploaded_file_good(&body, 1000, persist),
            "kept for others now"
        );
        assert!(!uploaded_file_good(&json!({}), 1000, TO_CONTACT));
    }

    /// What is due for a picture's download, on a network where pictures come by themselves.
    fn due_downloads(store: &Store, now: i64) -> Vec<(i64, MediaPart)> {
        store
            .transfers_to_retry(now, &["image".to_owned()], 0)
            .unwrap()
            .downloads
    }

    fn failed_download(
        app: &App,
        job: &Download,
        error: media::MediaError,
    ) -> mpsc::UnboundedReceiver<CoreEvent> {
        let (events, rx) = mpsc::unbounded_channel();
        assert!(!kept(job, app, &events, Err(error)));
        rx
    }

    /// A download the network broke off comes again by itself, after a pause, a limited number of
    /// times; a thumbnail as well as a file.
    #[test]
    fn a_failed_download_tries_again_a_limited_number_of_times() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        for part in [MediaPart::File, MediaPart::Thumbnail] {
            let job = job(&app.store, id, part);
            let field = state_field(part).trim_start_matches("$.");
            let mut now = now_ms();
            failed_download(&app, &job, media::MediaError::Network("timeout".into()));
            assert_eq!(state(&app.store, id, field), Some(STATE_FAILED));
            assert_eq!(due_downloads(&app.store, now), vec![]);
            for minutes in RETRY_PAUSES_MINUTES.iter().skip(1) {
                now += RETRY_PAUSE_MAX_MS;
                assert!(due_downloads(&app.store, now).contains(&(id, part)));
                app.store.set_media_failed(id, part, true, now).unwrap();
                assert_eq!(
                    app.store
                        .transfers_to_retry(now, &["image".to_owned()], 0)
                        .unwrap()
                        .next_at,
                    Some(now + minutes * MINUTE_MS)
                );
            }
            now += RETRY_PAUSE_MAX_MS;
            assert!(due_downloads(&app.store, now).contains(&(id, part)));
            app.store.set_media_failed(id, part, true, now).unwrap();
            assert!(!due_downloads(&app.store, now + RETRY_PAUSE_MAX_MS).contains(&(id, part)));
        }
    }

    /// What can't work later stays failed until it is tapped: a blob that is gone, one that opens
    /// with no key, one too large, a disk that is full.
    #[test]
    fn a_download_that_cannot_work_later_waits_for_a_tap() {
        let (_dir, app) = app();
        for (message_id, error) in [
            (1, media::MediaError::Status(404)),
            (2, media::MediaError::Decrypt),
            (3, media::MediaError::TooLarge),
            (
                4,
                media::MediaError::Io(std::io::ErrorKind::StorageFull.into()),
            ),
        ] {
            let id = received_file(&app.store, message_id, 1);
            failed_download(&app, &job(&app.store, id, MediaPart::File), error);
            assert_eq!(state(&app.store, id, "fileState"), Some(STATE_FAILED));
        }
        assert_eq!(due_downloads(&app.store, i64::MAX / 2), vec![]);
        assert_eq!(
            app.store
                .transfers_to_retry(now_ms() + RETRY_PAUSE_MAX_MS, &["image".to_owned()], 0)
                .unwrap(),
            TransferRetries::default()
        );
    }

    /// The settings for what comes by itself count for a try again as well: not a kind left out,
    /// not over the limit, not a file the user cleaned up. Its thumbnail still comes.
    #[test]
    fn a_failed_file_comes_again_only_as_it_would_by_itself() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        let now = now_ms();
        app.store
            .set_media_failed(id, MediaPart::File, true, now)
            .unwrap();
        app.store
            .set_media_failed(id, MediaPart::Thumbnail, true, now)
            .unwrap();
        let later = now + RETRY_PAUSE_MAX_MS;
        let downloads = |auto: &[String], limit| {
            app.store
                .transfers_to_retry(later, auto, limit)
                .unwrap()
                .downloads
        };
        let images = ["image".to_owned()];
        let both = vec![(id, MediaPart::Thumbnail), (id, MediaPart::File)];
        let thumbnail = vec![(id, MediaPart::Thumbnail)];
        assert_eq!(downloads(&images, 0), both);
        assert_eq!(downloads(&images, 1000), both);
        assert_eq!(downloads(&images, 999), thumbnail);
        assert_eq!(downloads(&["video".to_owned()], 0), thumbnail);
        app.store.set_file_cleaned(id).unwrap();
        assert_eq!(downloads(&images, 0), thumbnail);
    }

    /// Nothing changes for what failed before tries were counted, or what a restored backup marks
    /// as failed: there may be thousands, and they wait for a tap.
    #[test]
    fn a_download_without_a_note_waits_for_a_tap() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        app.store
            .set_media_state(id, MediaPart::File, STATE_FAILED)
            .unwrap();
        assert_eq!(due_downloads(&app.store, i64::MAX / 2), vec![]);
    }

    /// Fetched after all: the note goes. A tap starts the tries over.
    #[tokio::test(flavor = "current_thread")]
    async fn a_tap_on_a_download_starts_its_tries_over() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                let id = received_file(&app.store, 1, 1);
                let now = now_ms();
                for _ in 0..=RETRY_PAUSES_MINUTES.len() {
                    app.store
                        .set_media_failed(id, MediaPart::File, true, now)
                        .unwrap();
                }
                assert_eq!(due_downloads(&app.store, now + RETRY_PAUSE_MAX_MS), vec![]);
                app.download_file(id).unwrap();
                assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOADING));

                let failed = app.store.set_media_failed(id, MediaPart::File, true, now);
                let body = failed.unwrap().unwrap().body;
                let body: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(body["fileRetry"]["tries"], 1);

                app.store
                    .set_media_state(id, MediaPart::File, STATE_LOCAL)
                    .unwrap();
                let body = app.store.message(id).unwrap().unwrap().body;
                assert!(!body.contains("fileRetry"), "{body}");
            })
            .await;
    }

    /// Once it runs again it isn't due again, however often the list is looked at; and it stays
    /// under way as far as a new look at the list is concerned.
    #[tokio::test(flavor = "current_thread")]
    async fn a_retried_download_starts_once() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                app.connection_state.set("connected");
                let id = received_file(&app.store, 1, 1);
                app.store
                    .set_media_state(id, MediaPart::File, STATE_LOCAL)
                    .unwrap();
                app.store
                    .set_media_failed(id, MediaPart::Thumbnail, true, 0)
                    .unwrap();
                app.retry_failed_transfers();
                assert_eq!(state(&app.store, id, "thumbnailState"), Some(STATE_LOADING));
                assert_eq!(due_downloads(&app.store, now_ms()), vec![]);
                app.retry_failed_transfers();
                assert_eq!(due_downloads(&app.store, now_ms()), vec![]);
            })
            .await;
    }

    /// A file left by a download that outlived its message's deletion goes at the next start.
    #[test]
    fn deleted_messages_keep_no_media() {
        let (_dir, app) = app();
        let id = received_file(&app.store, 1, 1);
        assert!(app.store.message_keeps_media(id).unwrap());
        app.store.mark_deleted(id, 5).unwrap();
        assert!(app.store.message_exists(id).unwrap());
        assert!(!app.store.message_keeps_media(id).unwrap());
    }

    /// Played on the media page, the open chat below it has to stop showing it as new.
    #[tokio::test(flavor = "current_thread")]
    async fn a_voice_message_heard_is_news_for_every_page() {
        let (_dir, mut app) = app();
        let mut events = app.take_media_events().unwrap();
        let body =
            json!({"mediaType": "audio/aac", "rendering": "media", "fileState": STATE_LOCAL});
        let voice = app
            .store
            .insert_incoming("BBBBBBBB", 1, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap();
        let request = json!({"id": 1, "cmd": "setVoiceListened", "message": voice.id});
        let reply: Value = serde_json::from_str(&app.handle(&request.to_string()).await).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        let Ok(CoreEvent::MessagesChanged(changed)) = events.try_recv() else {
            panic!("no event for the change");
        };
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].id, voice.id);
        let heard: Value = serde_json::from_str(&changed[0].body).unwrap();
        assert_eq!(heard["listened"], true);
    }

    /// What a video sent as media says about itself stays behind; a file is sent as it is.
    #[tokio::test(flavor = "current_thread")]
    async fn a_media_video_leaves_without_its_metadata() {
        use crate::mp4::fixture;

        let (dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let (recording, samples) = fixture::video(1920, 1080, true);
        let source = dir.path().join("VID_20261105_120000.mp4");
        std::fs::write(&source, &recording).unwrap();
        let file = |rendering: &str, media_type: &str, keep_metadata: bool| OutgoingFile {
            identity: "BBBBBBBB".to_owned(),
            path: Some(source.to_string_lossy().into_owned()),
            name: Some("VID_20261105_120000.mp4".to_owned()),
            media_type: media_type.to_owned(),
            caption: None,
            rendering: rendering.to_owned(),
            width: Some(512),
            height: Some(512),
            duration: None,
            encode: None,
            keep_metadata,
        };
        let stored = |app: &App, reply: &Value| {
            let id = reply["id"].as_i64().unwrap();
            let body = app.store.message(id).unwrap().unwrap().body;
            let body: Value = serde_json::from_str(&body).unwrap();
            let bytes = app.media.read(id, MediaPart::File).unwrap().unwrap();
            (body, bytes.to_vec())
        };
        let contains = |data: &[u8], needle: &[u8]| data.windows(needle.len()).any(|w| w == needle);

        // Ends without yielding: the upload's task doesn't run, so nothing goes to the network.
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let reply = in_own_set(app.send_file(file("media", "video/mp4", false)))
                    .await
                    .unwrap();
                let (body, bytes) = stored(&app, &reply);
                for secret in fixture::SECRETS {
                    assert!(!contains(&bytes, secret));
                }
                assert_eq!(bytes.len(), recording.len());
                assert_eq!(
                    &bytes[samples..][..fixture::PICTURE.len()],
                    fixture::PICTURE
                );
                assert_eq!(body["rendering"], "media");
                // The real size as shown (filmed upright), not the thumbnail's.
                assert_eq!(
                    (&body["width"], &body["height"]),
                    (&json!(1080), &json!(1920))
                );
                assert_eq!(body["name"], "video.mp4");
                assert_eq!(body["size"], bytes.len());

                // Sent as a file, it is the user's file, unchanged.
                let reply = in_own_set(app.send_file(file("file", "video/mp4", false)))
                    .await
                    .unwrap();
                let (body, bytes) = stored(&app, &reply);
                assert_eq!(bytes, recording);
                assert_eq!(body["rendering"], "file");
                assert_eq!(body["name"], "VID_20261105_120000.mp4");

                // A photo sent as media says nothing with its name either.
                std::fs::write(&source, b"\xff\xd8\xff").unwrap();
                let reply = in_own_set(app.send_file(file("media", "image/jpeg", false)))
                    .await
                    .unwrap();
                assert_eq!(stored(&app, &reply).0["name"], "image.jpg");
            })
            .await;
    }

    /// What can't be cleaned doesn't leave unless the user says so, and nothing of it stays.
    #[tokio::test(flavor = "current_thread")]
    async fn a_video_that_cant_be_cleaned_waits_for_the_user() {
        let (dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let webm = b"\x1aE\xdf\xa3 a webm file";
        let source = dir.path().join("VID_20261105_120000.webm");
        std::fs::write(&source, webm).unwrap();
        let request = |keep: Option<bool>| {
            let mut request = json!({
                "id": 1, "cmd": "sendFile", "identity": "BBBBBBBB", "rendering": "media",
                "mediaType": "video/webm", "name": "VID_20261105_120000.webm",
                "path": source.to_string_lossy(),
            });
            if let Some(keep) = keep {
                request["keepMetadata"] = json!(keep);
            }
            request.to_string()
        };

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                for keep in [None, Some(false)] {
                    let reply: Value =
                        serde_json::from_str(&app.handle(&request(keep)).await).unwrap();
                    assert_eq!(reply["error"]["kind"], "videoKeepsMetadata", "{reply}");
                }
                assert!(
                    app.store
                        .conversation_messages("BBBBBBBB", 10)
                        .unwrap()
                        .is_empty(),
                    "nothing stored, nothing on its way"
                );

                let reply: Value =
                    serde_json::from_str(&app.handle(&request(Some(true))).await).unwrap();
                assert_eq!(reply["ok"], true, "{reply}");
                let id = reply["result"]["id"].as_i64().unwrap();
                let body = app.store.message(id).unwrap().unwrap().body;
                let body: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(body["rendering"], "file");
                assert_eq!(body["name"], "VID_20261105_120000.webm");
                assert_eq!(body["width"], Value::Null);
                let bytes = app.media.read(id, MediaPart::File).unwrap().unwrap();
                assert_eq!(bytes.as_slice(), webm);
            })
            .await;
    }

    /// Someone else's video that can't be cleaned isn't passed on with what it says either.
    #[tokio::test(flavor = "current_thread")]
    async fn forwarding_a_video_that_cant_be_cleaned_is_refused() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        for identity in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
        }
        let body = json!({"rendering": "media", "mediaType": "video/webm", "name": "a.webm",
                          "size": 4, "fileState": STATE_LOCAL});
        let received = app
            .store
            .insert_incoming("BBBBBBBB", 1, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap();
        app.media
            .write(received.id, MediaPart::File, b"\x1aE\xdf\xa3")
            .unwrap();
        assert!(matches!(
            in_own_set(app.forward_message(received.id, "CCCCCCCC", None)).await,
            Err(ApiError::VideoKeepsMetadata)
        ));
    }

    /// A forwarded picture doesn't pass on what its file says, whoever's it is, nor its size change.
    #[tokio::test(flavor = "current_thread")]
    async fn forwarding_a_received_picture_leaves_its_metadata_behind() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        for identity in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
        }
        let secret = images::fixture::SECRET;
        let contains = |data: &[u8], part: &[u8]| data.windows(part.len()).any(|w| w == part);
        let jpeg = images::fixture::jpeg_with_exif();
        let received = |app: &App, id: u64, media_type: &str, rendering: &str, data: &[u8]| {
            let body = json!({"rendering": rendering, "mediaType": media_type, "name": "a",
                              "size": data.len(), "fileState": STATE_LOCAL});
            let message = app
                .store
                .insert_incoming("BBBBBBBB", id, "file", &body.to_string(), 1)
                .unwrap()
                .unwrap();
            app.media.write(message.id, MediaPart::File, data).unwrap();
            message.id
        };
        let forward = async |app: &mut App, id: i64| -> Result<Value, ApiError> {
            let local = tokio::task::LocalSet::new();
            let sent = local
                .run_until(app.forward_message(id, "CCCCCCCC", None))
                .await?;
            let id = sent["id"].as_i64().unwrap();
            let body = app.store.message(id).unwrap().unwrap().body;
            let mut body: Value = serde_json::from_str(&body).unwrap();
            let data = app.media.read(id, MediaPart::File).unwrap().unwrap();
            body["sent"] = data_encoding::HEXLOWER.encode(&data).into();
            Ok(body)
        };
        let bytes = |body: &Value| {
            data_encoding::HEXLOWER
                .decode(body["sent"].as_str().unwrap().as_bytes())
                .unwrap()
        };

        let id = received(&app, 1, "image/jpeg", "media", &jpeg);
        let body = forward(&mut app, id).await.unwrap();
        assert!(!contains(&bytes(&body), secret));
        assert_eq!((&body["width"], &body["height"]), (&json!(10), &json!(20)));
        assert_eq!(body["thumbnail"], true);

        let webp = crate::webp::fixture::still();
        let id = received(&app, 2, "image/webp", "media", &webp);
        let body = forward(&mut app, id).await.unwrap();
        assert!(!contains(&bytes(&body), b"GPS here"));

        // A file is what the sender made of it, and stays as it was, as does a GIF.
        let id = received(&app, 3, "image/jpeg", "file", &jpeg);
        let body = forward(&mut app, id).await.unwrap();
        assert_eq!(bytes(&body), jpeg);
        let gif = crate::gif::fixture::with_metadata();
        let id = received(&app, 4, "image/gif", "media", &gif);
        let body = forward(&mut app, id).await.unwrap();
        assert_eq!(bytes(&body), crate::gif::fixture::clean());

        // What can't be read isn't passed on.
        let damaged = [&jpeg[..40], b"cut"].concat();
        for (message, media_type, data) in [
            (5, "image/jpeg", damaged.as_slice()),
            (6, "image/heic", b"a heic".as_slice()),
        ] {
            let id = received(&app, message, media_type, "media", data);
            assert!(matches!(
                forward(&mut app, id).await,
                Err(ApiError::PictureKeepsMetadata)
            ));
        }

        // By its bytes, whatever it is labelled.
        for (message, media_type) in [(8, "IMAGE/JPEG"), (9, "application/octet-stream")] {
            let id = received(&app, message, media_type, "media", &jpeg);
            let body = forward(&mut app, id).await.unwrap();
            assert!(!contains(&bytes(&body), secret), "{media_type}");
            assert_eq!(body["mediaType"], "image/jpeg", "{media_type}");
        }

        // It keeps its size.
        let large = {
            let image = image::RgbImage::from_pixel(2000, 1000, image::Rgb([1, 2, 3]));
            let mut out = std::io::Cursor::new(Vec::new());
            image.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
            out.into_inner()
        };
        let id = received(&app, 10, "image/jpeg", "media", &large);
        let body = forward(&mut app, id).await.unwrap();
        assert_eq!(body["width"], 2000);

        // A thumbnail of the sender's, for what stays as it is, is made anew or left behind.
        let id = received(&app, 11, "image/gif", "media", &gif);
        app.media.write(id, MediaPart::Thumbnail, &jpeg).unwrap();
        let body = forward(&mut app, id).await.unwrap();
        assert_eq!(body["thumbnail"], true);
        let copy = app
            .store
            .conversation_messages("CCCCCCCC", 1)
            .unwrap()
            .remove(0)
            .id;
        let made = app.media.read(copy, MediaPart::Thumbnail).unwrap().unwrap();
        assert!(!contains(&made, secret));
        let id = received(&app, 12, "image/gif", "media", &gif);
        app.media
            .write(id, MediaPart::Thumbnail, b"no picture, GPS-SECRET")
            .unwrap();
        let body = forward(&mut app, id).await.unwrap();
        assert_eq!(body["thumbnail"], false);

        // Not a picture, by label or by its bytes: nothing to clean.
        let id = received(&app, 13, "application/pdf", "media", b"%PDF");
        assert_eq!(bytes(&forward(&mut app, id).await.unwrap()), b"%PDF");

        // Ours too: one may have come from a linked device.
        let own = app
            .store
            .insert_outgoing(
                "BBBBBBBB",
                7,
                "file",
                &json!({"rendering": "media", "mediaType": "image/jpeg", "name": "image.jpg",
                        "size": 9, "fileState": STATE_LOCAL})
                .to_string(),
            )
            .unwrap();
        app.media.write(own.id, MediaPart::File, &jpeg).unwrap();
        let body = forward(&mut app, own.id).await.unwrap();
        assert!(!contains(&bytes(&body), secret));

        // A list's members get what the list got, not a further generation of it.
        let list = app
            .store
            .create_distribution_list("Crew", vec!["CCCCCCCC".into()])
            .unwrap();
        let id = received(&app, 14, "image/jpeg", "media", &jpeg);
        let sent = in_own_set(app.forward_message(id, &list.key, None))
            .await
            .unwrap();
        let list_copy = sent["id"].as_i64().unwrap();
        let of_the_list = app.media.read(list_copy, MediaPart::File).unwrap().unwrap();
        assert!(!contains(&of_the_list, secret));
        let of_the_member = app
            .store
            .conversation_messages("CCCCCCCC", 1)
            .unwrap()
            .remove(0)
            .id;
        assert_eq!(
            app.media
                .read(of_the_member, MediaPart::File)
                .unwrap()
                .unwrap()
                .as_slice(),
            of_the_list.as_slice()
        );
    }

    /// The user can change or take away a file's caption when forwarding it; a text stays itself.
    #[tokio::test(flavor = "current_thread")]
    async fn a_forward_can_change_or_drop_the_caption() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        for identity in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
        }
        let body = json!({"rendering": "file", "mediaType": "application/pdf", "name": "a.pdf",
                          "size": 4, "caption": "the first words", "fileState": STATE_LOCAL});
        let file = app
            .store
            .insert_incoming("BBBBBBBB", 1, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap()
            .id;
        app.media.write(file, MediaPart::File, b"%PDF").unwrap();
        let text = app
            .store
            .insert_incoming("BBBBBBBB", 2, "text", "only words", 1)
            .unwrap()
            .unwrap()
            .id;
        let mut forward = async |extra: Value| -> Value {
            let mut request = json!({"id": 1, "cmd": "forwardMessages", "identity": "CCCCCCCC",
                                     "messages": [file]});
            for (key, value) in extra.as_object().unwrap() {
                request[key] = value.clone();
            }
            let reply: Value =
                serde_json::from_str(&in_own_set(app.handle(&request.to_string())).await).unwrap();
            assert_eq!(reply["result"]["forwarded"], 1, "{reply}");
            let newest = app.store.conversation_messages("CCCCCCCC", 1).unwrap();
            serde_json::from_str(&newest[0].body).unwrap_or(newest[0].body.clone().into())
        };
        assert_eq!(forward(json!({})).await["caption"], "the first words");
        assert_eq!(
            forward(json!({"caption": "my words"})).await["caption"],
            "my words"
        );
        assert_eq!(
            forward(json!({"caption": ""})).await["caption"],
            Value::Null
        );
        assert_eq!(
            forward(json!({"messages": [text], "caption": "my words"})).await,
            Value::from("only words")
        );
    }

    /// A member a list's file couldn't reach is counted, and which or why stays out of the answer.
    #[tokio::test(flavor = "current_thread")]
    async fn a_forward_to_a_list_counts_the_members_it_missed() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        for identity in ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
        }
        let body = json!({"rendering": "file", "mediaType": "application/pdf", "name": "a.pdf",
                          "size": 4, "fileState": STATE_LOCAL});
        let file = app
            .store
            .insert_incoming("BBBBBBBB", 1, "file", &body.to_string(), 1)
            .unwrap()
            .unwrap()
            .id;
        app.media.write(file, MediaPart::File, b"%PDF").unwrap();
        let list = app
            .store
            .create_distribution_list("Crew", vec!["CCCCCCCC".into(), "DDDDDDDD".into()])
            .unwrap();
        let mut forward = async || -> Value {
            let request = json!({"id": 1, "cmd": "forwardMessages", "identity": list.key,
                                 "messages": [file]})
            .to_string();
            let reply = in_own_set(app.handle(&request)).await;
            serde_json::from_str(&reply).unwrap()
        };
        let reply = forward().await;
        assert_eq!(reply["result"]["forwarded"], 1, "{reply}");
        assert_eq!(reply["result"]["failedMembers"], 0, "{reply}");

        app.store.set_private("DDDDDDDD", true).unwrap();
        app.unlocked.set(false);
        let mut forward = async || -> Value {
            let request = json!({"id": 1, "cmd": "forwardMessages", "identity": list.key,
                                 "messages": [file]})
            .to_string();
            let reply = in_own_set(app.handle(&request)).await;
            serde_json::from_str(&reply).unwrap()
        };
        let reply = forward().await;
        assert_eq!(reply["result"]["forwarded"], 1, "{reply}");
        assert_eq!(reply["result"]["failedMembers"], 1, "{reply}");
        assert_eq!(reply["result"]["errors"], json!([]), "{reply}");
    }

    /// What couldn't be forwarded is said, not only counted: the UI has only this to go by.
    #[tokio::test(flavor = "current_thread")]
    async fn forwarding_several_tells_why_some_failed() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        for identity in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
        }
        let store_file = |id: u64| {
            let body = json!({"rendering": "media", "mediaType": "image/heic", "name": "a",
                              "size": 6, "fileState": STATE_LOCAL});
            let message = app
                .store
                .insert_incoming("BBBBBBBB", id, "file", &body.to_string(), 1)
                .unwrap()
                .unwrap();
            app.media
                .write(message.id, MediaPart::File, b"a heic")
                .unwrap();
            message.id
        };
        let (one, two) = (store_file(1), store_file(2));
        let text = app
            .store
            .insert_incoming("BBBBBBBB", 3, "text", "hello", 1)
            .unwrap()
            .unwrap()
            .id;
        let request = json!({"id": 1, "cmd": "forwardMessages", "identity": "CCCCCCCC",
                             "messages": [one, text, two, 999_999]})
        .to_string();
        let reply: Value = serde_json::from_str(&in_own_set(app.handle(&request)).await).unwrap();
        assert_eq!(reply["result"]["forwarded"], 1, "{reply}");
        assert_eq!(reply["result"]["failed"], 3, "{reply}");
        assert_eq!(
            reply["result"]["errors"],
            json!(["pictureKeepsMetadata", "internal"]),
            "{reply}"
        );
    }

    /// Every way of writing brings an archived chat back, a list's copies their members' chats too.
    #[tokio::test(flavor = "current_thread")]
    async fn writing_brings_a_chat_back_from_the_archive() {
        let (_dir, mut app) = app();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        for identity in [
            "BBBBBBBB", "CCCCCCCC", "DDDDDDDD", "EEEEEEEE", "FFFFFFFF", "GGGGGGGG",
        ] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(identity))
                .unwrap();
        }
        let group = crate::groups::Group {
            key: "g:ABCD1234:0000000000000001".to_owned(),
            identity: libthreema::common::GroupIdentity {
                creator_identity: "ABCD1234".parse().unwrap(),
                group_id: 1,
            },
            name: "g".into(),
            members: vec!["BBBBBBBB".to_owned()],
            left: false,
            created_at: 0,
            picture: None,
            picture_version: 0,
        };
        app.store.upsert_group(&group).unwrap();
        let list = app
            .store
            .create_distribution_list("l", vec!["GGGGGGGG".to_owned()])
            .unwrap();
        let archive = |app: &App, key: &str| app.store.set_archived(key, true).unwrap();
        let back =
            |app: &App, key: &str| !app.store.archived_chats().unwrap().iter().any(|k| k == key);

        archive(&app, "BBBBBBBB");
        app.send_text("BBBBBBBB", "hello", None).unwrap();
        assert!(back(&app, "BBBBBBBB"), "text");

        archive(&app, "CCCCCCCC");
        let place = json!({"latitude": 1.0, "longitude": 2.0});
        app.send_location("CCCCCCCC", &place).unwrap();
        assert!(back(&app, "CCCCCCCC"), "location");

        archive(&app, "DDDDDDDD");
        let choices = ["a".to_owned(), "b".to_owned()];
        app.create_poll("DDDDDDDD", "?", &choices, false, false)
            .unwrap();
        assert!(back(&app, "DDDDDDDD"), "poll");

        archive(&app, &group.key);
        app.send_text(&group.key, "hello", None).unwrap();
        assert!(back(&app, &group.key), "group");

        archive(&app, &list.key);
        archive(&app, "GGGGGGGG");
        app.send_text(&list.key, "hello", None).unwrap();
        assert!(back(&app, &list.key), "list");
        assert!(back(&app, "GGGGGGGG"), "a list's member");

        let source = app.store.conversation_messages("BBBBBBBB", 1).unwrap()[0].id;
        archive(&app, "EEEEEEEE");
        in_own_set(app.forward_message(source, "EEEEEEEE", None))
            .await
            .unwrap();
        assert!(back(&app, "EEEEEEEE"), "forward");

        // Ends without yielding: the upload's task doesn't run, so nothing goes to the network.
        archive(&app, "FFFFFFFF");
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                app.attachments = vec![Zeroizing::new(b"bytes".to_vec())];
                in_own_set(app.send_file(OutgoingFile {
                    identity: "FFFFFFFF".to_owned(),
                    path: None,
                    name: Some("a.bin".to_owned()),
                    media_type: "application/octet-stream".to_owned(),
                    caption: None,
                    rendering: "file".to_owned(),
                    width: None,
                    height: None,
                    duration: None,
                    encode: None,
                    keep_metadata: false,
                }))
                .await
                .unwrap();
            })
            .await;
        assert!(back(&app, "FFFFFFFF"), "file");
        assert_eq!(app.unarchived.borrow().len(), 8);

        // Others stay: receiving and reflected messages aren't this path, nor is an untouched chat.
        archive(&app, "CCCCCCCC");
        app.send_text("BBBBBBBB", "again", None).unwrap();
        assert!(!back(&app, "CCCCCCCC"));
    }

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

    /// Old rows for the tests below: these messages also reached the device when they say.
    fn arrived_when_written(store: &Store) {
        store
            .conn
            .execute(
                "UPDATE messages SET received_at = created_at WHERE received_at IS NOT NULL",
                [],
            )
            .unwrap();
    }

    /// An incoming `created_at` is the sender's clock: a backdated message, or one held back for
    /// days, is new to this device and stays as long as any other.
    #[test]
    fn age_goes_by_arrival_not_by_the_senders_clock() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let now = crate::messages::now_ms();
        let body = json!({"mediaType": "image/jpeg", "fileState": STATE_LOCAL}).to_string();
        let backdated_file = store
            .insert_incoming("AAAAAAAA", 1, "file", &body, 1_000)
            .unwrap()
            .unwrap();
        with_blob(&store, backdated_file.id, Some(1));
        let backdated = store
            .insert_incoming("AAAAAAAA", 2, "text", "backdated", 1_000)
            .unwrap()
            .unwrap();
        let week = 7 * 86_400_000;
        assert!(store.files_before(now - week, &[]).unwrap().is_empty());
        assert_eq!(
            store
                .delete_messages_before(now - week, false, &[])
                .unwrap(),
            0
        );
        assert!(store.message(backdated.id).unwrap().is_some());

        // Dated ahead: it doesn't outstay the others either.
        let future = store
            .insert_incoming("AAAAAAAA", 3, "text", "ahead", now + 365 * 86_400_000)
            .unwrap()
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET received_at = ?1 WHERE id = ?2",
                params![now - 2 * week, future.id],
            )
            .unwrap();
        assert_eq!(
            store
                .delete_messages_before(now - week, false, &[])
                .unwrap(),
            1
        );
        assert!(store.message(future.id).unwrap().is_none());

        // Rows without an arrival time (ours, imported ones) go by their only clock.
        let own = store
            .insert_outgoing("AAAAAAAA", 4, "text", "sent long ago")
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET created_at = ?1, state = ?2 WHERE id = ?3",
                params![
                    now - 2 * week,
                    crate::messages::OutgoingState::Sent as i64,
                    own.id
                ],
            )
            .unwrap();
        assert_eq!(
            store
                .delete_messages_before(now - week, false, &[])
                .unwrap(),
            1
        );
        assert!(store.message(own.id).unwrap().is_none());
    }

    /// Clean up leaves private chats alone while they are hidden, and doesn't count them: it needs
    /// no PIN when the app lock is off. Unlocked, they are chats like any other.
    #[test]
    fn clean_up_leaves_hidden_private_chats_alone() {
        let (_dir, mut app) = app();
        let body = json!({"mediaType": "image/jpeg", "fileState": STATE_LOCAL}).to_string();
        let mut files = Vec::new();
        for (id, chat) in [(1, "AAAAAAAA"), (3, "BBBBBBBB")] {
            let file = app
                .store
                .insert_incoming(chat, id, "file", &body, 1_000)
                .unwrap()
                .unwrap();
            with_blob(&app.store, file.id, Some(1));
            files.push(file.id);
            app.store
                .insert_incoming(chat, id + 1, "text", "old", 1_000)
                .unwrap();
        }
        arrived_when_written(&app.store);
        app.store.set_private("AAAAAAAA", true).unwrap();
        assert!(!app.unlocked.get());

        assert_eq!(app.clean_up(7, false).unwrap()["removed"], 1);
        assert_eq!(state(&app.store, files[0], "fileState"), Some(STATE_LOCAL));
        assert_eq!(state(&app.store, files[1], "fileState"), Some(STATE_REMOTE));
        assert_eq!(app.clean_up(7, true).unwrap()["removed"], 2);
        assert_eq!(app.store.message_count("AAAAAAAA").unwrap(), 2);
        assert_eq!(app.store.message_count("BBBBBBBB").unwrap(), 0);

        app.unlocked.set(true);
        assert_eq!(app.clean_up(7, false).unwrap()["removed"], 1);
        assert_eq!(app.clean_up(7, true).unwrap()["removed"], 2);
        assert_eq!(app.store.message_count("AAAAAAAA").unwrap(), 0);
    }

    /// A pinned chat or one with a draft stays listed when its last message ages out, in the
    /// place that message gave it. Another 1:1 chat goes.
    #[test]
    fn a_pinned_or_drafted_chat_stays_listed_without_messages() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        for (id, chat) in ["AAAAAAAA", "BBBBBBBB", "CCCCCCCC", "DDDDDDDD"]
            .into_iter()
            .enumerate()
        {
            store
                .insert_incoming(chat, id as u64 + 1, "text", "old", 1_000 + id as i64)
                .unwrap();
        }
        // Still has a row after the deletion: no stand-in needed.
        let starred = store
            .insert_incoming("DDDDDDDD", 9, "text", "keep", 1_000)
            .unwrap()
            .unwrap();
        store.set_starred(starred.id, true).unwrap();
        arrived_when_written(&store);
        store.set_pinned("AAAAAAAA", true).unwrap();
        store.set_pinned("DDDDDDDD", true).unwrap();
        // Pinned with nothing to lose: it doesn't start showing up.
        store.set_pinned("EEEEEEEE", true).unwrap();
        store.set_setting("draft:BBBBBBBB", "a thought").unwrap();

        assert_eq!(store.delete_messages_before(5_000, false, &[]).unwrap(), 4);

        let mut listed: Vec<(String, String, i64)> = store
            .conversations()
            .unwrap()
            .into_iter()
            .map(|(m, _)| (m.conversation.clone(), m.kind.clone(), m.created_at))
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            [
                ("AAAAAAAA".to_owned(), "empty".to_owned(), 1_000),
                ("BBBBBBBB".to_owned(), "empty".to_owned(), 1_001),
                ("DDDDDDDD".to_owned(), "text".to_owned(), 1_000),
            ]
        );
    }

    /// This phone's clock was far behind when a message came, so its arrival time is long past
    /// once the clock is right. The message's own date, not in the future, shows it is new.
    #[test]
    fn a_message_received_under_a_clock_far_behind_stays() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let now = crate::messages::now_ms();
        let day = 86_400_000;
        let week = 7 * day;
        let insert = |id: u64, written: i64, arrived: i64| {
            let message = store
                .insert_incoming("AAAAAAAA", id, "text", "x", written)
                .unwrap()
                .unwrap();
            store
                .conn
                .execute(
                    "UPDATE messages SET received_at = ?1 WHERE id = ?2",
                    params![arrived, message.id],
                )
                .unwrap();
            message.id
        };
        let behind = insert(1, now - 3_600_000, now - 365 * day);
        let behind_but_old = insert(2, now - 2 * week, now - 365 * day);
        let ahead_of_now = insert(3, now + 365 * day, now - 2 * week);
        let small_skew = insert(4, now - 2 * week + 3_600_000, now - 2 * week);
        assert_eq!(
            store
                .delete_messages_before(now - week, false, &[])
                .unwrap(),
            3
        );
        assert!(store.message(behind).unwrap().is_some());
        for gone in [behind_but_old, ahead_of_now, small_skew] {
            assert!(store.message(gone).unwrap().is_none());
        }
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
        arrived_when_written(&store);
        // A call long ago: its status line goes with the rest.
        let call = store
            .insert_status("AAAAAAAA", "call|finished|1|65")
            .unwrap()
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE messages SET created_at = ?1 WHERE id = ?2",
                params![old, call.id],
            )
            .unwrap();

        assert_eq!(
            store.delete_messages_before(now - 1, false, &[]).unwrap(),
            3
        );
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

    /// Clean up gives the space back, and what stays is still all there: the figure in Settings
    /// counts free pages too, so without this it wouldn't move.
    #[test]
    fn cleaning_up_messages_shrinks_the_database() {
        let (_dir, mut app) = app();
        let text = "old news ".repeat(200);
        for id in 1..=500 {
            app.store
                .insert_incoming("ABCD1234", id, "text", &text, 1_000)
                .unwrap();
        }
        let kept = app
            .store
            .insert_incoming("ABCD1234", 501, "text", "starred harbour", 1_000)
            .unwrap()
            .unwrap();
        app.store.set_starred(kept.id, true).unwrap();
        arrived_when_written(&app.store);

        // Media only: no messages go, nothing to give back.
        let before = app.store.size().unwrap();
        app.clean_up(7, false).unwrap();
        assert_eq!(app.store.size().unwrap(), before);

        // A few messages out of many: not worth rewriting the file.
        app.store
            .conn
            .execute(
                "UPDATE messages SET received_at = ?1 WHERE message_id > 1",
                [crate::messages::now_ms()],
            )
            .unwrap();
        assert_eq!(app.clean_up(7, true).unwrap()["removed"], 1);
        assert_eq!(app.store.size().unwrap(), before);
        arrived_when_written(&app.store);

        app.clean_up(7, true).unwrap();
        let after = app.store.size().unwrap();
        assert!(after < before / 4, "{before} went to {after}");
        assert!(app.store.message(kept.id).unwrap().is_some());
        let found = app
            .store
            .search_messages("harbour", None, 10, |_| true, |_| None)
            .unwrap();
        assert_eq!(found.len(), 1, "the search index survived");
    }

    /// A last run dated ahead (written while the clock was wrong) doesn't hold Keep messages back
    /// until that date; one from a few hours ago does hold it back.
    #[test]
    fn keep_messages_runs_after_a_last_run_dated_ahead() {
        let (_dir, mut app) = app();
        let now = crate::messages::now_ms();
        let old = app
            .store
            .insert_incoming("ABCD1234", 1, "text", "old", 1_000)
            .unwrap()
            .unwrap();
        arrived_when_written(&app.store);
        app.store.set_setting(keys::AUTO_DELETE_DAYS, "7").unwrap();

        app.store
            .set_setting(keys::AUTO_DELETE_LAST, &(now - 3_600_000).to_string())
            .unwrap();
        assert!(!app.auto_delete_scheduled());
        assert!(app.store.message(old.id).unwrap().is_some());

        app.store
            .set_setting(
                keys::AUTO_DELETE_LAST,
                &(now + 365 * 86_400_000).to_string(),
            )
            .unwrap();
        assert!(app.auto_delete_scheduled());
        assert!(app.store.message(old.id).unwrap().is_none());
        let last: i64 = app
            .store
            .setting(keys::AUTO_DELETE_LAST)
            .unwrap()
            .unwrap()
            .parse()
            .unwrap();
        assert!(last < now + 86_400_000, "the stamp is back in the present");
    }

    /// Keep messages leaves notes groups alone, as Android's auto-delete does: nobody else has a
    /// copy. Clean up is asked for in every chat, and a group we left or one with members is an
    /// ordinary chat either way.
    #[test]
    fn keep_messages_spares_notes_groups() {
        let (_dir, mut app) = app();
        let group = |key: &str, members: Vec<&str>, left: bool| crate::groups::Group {
            key: key.into(),
            identity: libthreema::common::GroupIdentity {
                creator_identity: "ABCD1234".parse().unwrap(),
                group_id: 1,
            },
            name: String::new(),
            members: members.into_iter().map(str::to_owned).collect(),
            left,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        let notes = "g:ABCD1234:0000000000000001";
        let with_members = "g:ABCD1234:0000000000000002";
        let left = "g:ABCD1234:0000000000000003";
        app.store
            .upsert_group(&group(notes, Vec::new(), false))
            .unwrap();
        app.store
            .upsert_group(&group(with_members, vec!["ECHOECHO"], false))
            .unwrap();
        app.store
            .upsert_group(&group(left, Vec::new(), true))
            .unwrap();
        for (id, chat) in [notes, with_members, left, "ECHOECHO"]
            .into_iter()
            .enumerate()
        {
            let sent = app
                .store
                .insert_outgoing(chat, id as u64 + 1, "text", "old")
                .unwrap();
            app.store
                .conn
                .execute(
                    "UPDATE messages SET created_at = 1000, state = ?1 WHERE id = ?2",
                    params![crate::messages::OutgoingState::Sent as i64, sent.id],
                )
                .unwrap();
        }
        let chats_with_messages = |app: &App| {
            let mut chats: Vec<String> = app
                .store
                .conn
                .prepare("SELECT conversation FROM messages")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            chats.sort();
            chats
        };

        app.store.set_setting(keys::AUTO_DELETE_DAYS, "7").unwrap();
        assert!(app.auto_delete_scheduled());
        assert_eq!(
            chats_with_messages(&app),
            [notes],
            "only the notebook stays"
        );

        app.clean_up(7, true).unwrap();
        assert!(
            chats_with_messages(&app).is_empty(),
            "Clean up means every chat"
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

        assert_eq!(
            upload_target(&store, own.id),
            (media::BlobScope::Local, false)
        );
        // Every member fetches a group's blob, and a 1:1 message's is done once fetched.
        assert_eq!(
            upload_target(&store, theirs.id),
            (media::BlobScope::Public, true)
        );
        assert_eq!(
            upload_target(&store, contact.id),
            (media::BlobScope::Public, false)
        );
    }

    /// `done` lets the server drop a blob: without multi-device only for one we received 1:1, not
    /// for a group's, which the other members still need, nor for our own, which its recipient may
    /// still need when we fetch it again after a clean-up.
    #[test]
    fn only_a_received_one_to_one_blob_is_done_for_good() {
        use media::BlobScope::{Local, Public};
        // Without multi-device.
        assert_eq!(done_scope(false, false, false), Some(Public));
        assert_eq!(done_scope(true, false, false), None);
        assert_eq!(done_scope(false, true, false), None);
        assert_eq!(done_scope(true, true, false), None);
        // With it: the mirror keeps the blob for the other devices.
        assert_eq!(done_scope(false, false, true), Some(Public));
        assert_eq!(done_scope(true, false, true), Some(Local));
        assert_eq!(done_scope(false, true, true), Some(Local));
        assert_eq!(done_scope(true, true, true), Some(Local));
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

    /// Clean up → Media files frees the space for good: the file stays downloadable, by hand.
    #[tokio::test(flavor = "current_thread")]
    async fn cleaned_files_download_only_when_asked_for() {
        let (_dir, mut app) = app();
        let id = received_file(&app.store, 1, 1);
        app.store
            .set_media_state(id, MediaPart::File, STATE_LOCAL)
            .unwrap();
        app.store
            .set_media_state(id, MediaPart::Thumbnail, STATE_LOCAL)
            .unwrap();
        app.media.write(id, MediaPart::File, b"picture").unwrap();
        app.media.write(id, MediaPart::Thumbnail, b"thumb").unwrap();
        let every_kind = ["image".to_owned(), "file".to_owned()];

        let cleaned = app.clean_up(0, false).unwrap();
        assert_eq!(cleaned["removed"], 1);
        assert!(!app.media.has(id, MediaPart::File));
        assert!(app.media.has(id, MediaPart::Thumbnail));
        assert_eq!(state(&app.store, id, "fileState"), Some(STATE_REMOTE));
        assert!(
            app.store
                .media_to_download(&every_kind, 0)
                .unwrap()
                .is_empty()
        );
        let body = app.store.message(id).unwrap().unwrap().body;
        assert!(!awaits_download(&body));
        assert!(body.contains("a.jpg"), "the rest of the body stays");

        // Asked for: it comes. A restart during that doesn't turn it automatic.
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async { app.download_file(id).unwrap() })
            .await;
        assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOADING));
        app.store.reset_loading_media().unwrap();
        assert_eq!(state(&app.store, id, "fileState"), Some(STATE_REMOTE));
        assert!(
            app.store
                .media_to_download(&every_kind, 0)
                .unwrap()
                .is_empty()
        );
    }

    /// Blob keys of a file that went up, or (`blob`: `None`) of one with nothing on the server.
    fn with_blob(store: &Store, message: i64, blob: Option<u8>) {
        store
            .set_file_keys(
                message,
                &FileKeys {
                    key: Zeroizing::new([4; 32]),
                    blob_id: blob.map(|b| libthreema::common::BlobId([b; 16])),
                    thumbnail_blob_id: None,
                },
            )
            .unwrap();
    }

    /// A file still to go out keeps its bytes: its upload reads them.
    #[test]
    fn cleaning_up_leaves_unsent_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let body = json!({"mediaType": "video/mp4", "fileState": STATE_LOCAL}).to_string();
        let unsent = store.insert_outgoing("ABCD1234", 1, "file", &body).unwrap();
        let sent = store.insert_outgoing("ABCD1234", 2, "file", &body).unwrap();
        with_blob(&store, unsent.id, Some(1));
        with_blob(&store, sent.id, Some(2));
        store
            .raise_outgoing_state(
                "ABCD1234",
                &[sent.message_id],
                crate::messages::OutgoingState::Sent,
            )
            .unwrap();
        let cleaned = store.files_before(i64::MAX, &[]).unwrap();
        assert_eq!(cleaned, vec![sent.id]);
        assert!(!cleaned.contains(&unsent.id));
    }

    /// Clean up takes only files a server may still have: one with no blob stays, a list's own
    /// copy and a file without keys alike, since nothing could ever bring it back.
    #[test]
    fn cleaning_up_leaves_what_could_not_come_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let body = json!({"mediaType": "video/mp4", "fileState": STATE_LOCAL}).to_string();
        let insert = |id| {
            store
                .insert_incoming("ABCD1234", id, "file", &body, 1)
                .unwrap()
                .unwrap()
                .id
        };
        let without_keys = insert(1);
        let without_blob = insert(2);
        let with_one = insert(3);
        with_blob(&store, without_blob, None);
        with_blob(&store, with_one, Some(1));
        assert_eq!(store.files_before(i64::MAX, &[]).unwrap(), vec![with_one]);
        assert_eq!(
            state(&store, without_keys, "fileState"),
            Some(STATE_LOCAL),
            "stays as it was"
        );
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
        for message in [old.id, starred.id, recent.id] {
            with_blob(&store, message, Some(1));
        }
        arrived_when_written(&store);
        assert_eq!(store.files_before(5_000, &[]).unwrap(), vec![old.id]);
        assert_eq!(store.delete_messages_before(5_000, false, &[]).unwrap(), 2);
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
        // A sender's file without a name still opens by its type.
        for (media_type, name) in [
            ("video/mp4", "threesea-file.mp4"),
            ("video/quicktime", "threesea-file.mov"),
            ("image/gif", "threesea-file.gif"),
            ("image/webp", "threesea-file.webp"),
            ("audio/aac", "threesea-file.aac"),
            ("audio/ogg", "threesea-file.ogg"),
        ] {
            assert_eq!(safe_file_name(None, media_type), name);
        }

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

    /// A stopped upload isn't started again by anything but the user: not by the next connection,
    /// any pause, or a restart.
    #[tokio::test(flavor = "current_thread")]
    async fn a_cancelled_upload_waits_for_a_tap() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                app.connection_state.set("connected");
                app.store
                    .upsert_contact(&crate::contacts::test_contact("ABCD1234"))
                    .unwrap();
                let id = outgoing_file(&app.store, 1, 1);
                app.store.set_upload_state(id, STATE_FAILED).unwrap();
                app.retry_upload(id).unwrap();
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOADING));

                app.cancel_transfer(id).unwrap();
                app.transfers.ended().await;
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));

                let much_later = now_ms() + 1_000 * RETRY_PAUSE_MAX_MS;
                let retries = app.store.transfers_to_retry(much_later, &[], 0).unwrap();
                assert!(retries.uploads.is_empty());
                assert!(retries.next_at.is_none());
                app.retry_failed_transfers();
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_FAILED));
                assert!(app.store.unfinished_uploads().unwrap().is_empty());

                app.retry_upload(id).unwrap();
                assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOADING));
                app.cancel_transfer(id).unwrap();
            })
            .await;
    }

    /// A stopped download waits for a tap too, even of a kind that comes by itself.
    #[tokio::test(flavor = "current_thread")]
    async fn a_cancelled_download_waits_for_a_tap() {
        let (_dir, mut app) = app();
        tokio::task::LocalSet::new()
            .run_until(async {
                let id = received_file(&app.store, 1, 1);
                app.store
                    .set_media_state(id, MediaPart::Thumbnail, STATE_LOCAL)
                    .unwrap();
                let images = vec!["image".to_owned()];
                assert!(!app.store.media_to_download(&images, 0).unwrap().is_empty());
                app.download_file(id).unwrap();
                assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOADING));

                app.cancel_transfer(id).unwrap();
                app.transfers.ended().await;
                assert_eq!(state(&app.store, id, "fileState"), Some(STATE_REMOTE));
                assert!(app.store.media_to_download(&images, 0).unwrap().is_empty());
                assert!(
                    app.store
                        .transfers_to_retry(now_ms(), &images, 0)
                        .unwrap()
                        .downloads
                        .is_empty()
                );

                app.download_file(id).unwrap();
                assert_eq!(state(&app.store, id, "fileState"), Some(STATE_LOADING));
                app.cancel_transfer(id).unwrap();
            })
            .await;
    }

    /// What isn't running stays as it is: a file here isn't taken for one on its way.
    #[test]
    fn cancelling_changes_nothing_when_nothing_runs() {
        let (_dir, mut app) = app();
        let id = outgoing_file(&app.store, 1, 1);
        app.store.set_upload_state(id, STATE_LOCAL).unwrap();
        app.cancel_transfer(id).unwrap();
        assert_eq!(state(&app.store, id, "uploadState"), Some(STATE_LOCAL));
    }

    /// Stopping a transfer drops what it holds at once, and it no longer counts as running. The
    /// end of a task replaced meanwhile leaves its successor stoppable.
    #[tokio::test(flavor = "current_thread")]
    async fn stopping_a_transfer_lets_go_of_what_it_holds() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let transfers = RunningTransfers::default();
                let key = TransferKey::Upload(1);
                let held = Rc::new(());
                let in_task = Rc::clone(&held);
                transfers.spawn(key, async move {
                    let _held = in_task;
                    std::future::pending::<()>().await;
                });
                tokio::task::yield_now().await;
                assert_eq!(Rc::strong_count(&held), 2);
                transfers.abort(key);
                transfers.ended().await;
                assert_eq!(Rc::strong_count(&held), 1);
                assert!(transfers.none_running());

                transfers.spawn(key, async {});
                let in_task = Rc::clone(&held);
                transfers.spawn(key, async move {
                    let _held = in_task;
                    std::future::pending::<()>().await;
                });
                tokio::task::yield_now().await;
                assert!(transfers.tasks.borrow().contains_key(&key));
                transfers.abort(key);
                transfers.ended().await;
                assert_eq!(Rc::strong_count(&held), 1);
            })
            .await;
    }
}
